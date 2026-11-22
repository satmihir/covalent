import AVFoundation
import CovalentVoice
import Observation

/// VoiceSession is voice mode's microphone, speaker and server, wired to the turn-taking rules
/// in VoiceLoop. It listens with echo cancellation and reads each reply aloud as it is written,
/// until stopped. Because the app records and plays audio, iOS keeps it running with the phone
/// locked or another app in front.
@MainActor @Observable
final class VoiceSession {
    enum Status: String {
        case off = "Off"
        case starting = "Starting…"
        case listening = "Listening"
        case transcribing = "Transcribing…"
        case thinking = "Thinking…"
        case speaking = "Speaking"
        case paused = "Paused by another sound"
    }

    private(set) var status = Status.off
    private(set) var error: String?
    /// level is the microphone's loudness in decibels, for the meter.
    private(set) var level: Float = -100

    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private var detector = SpeechDetector(rate: 16000)
    private var loop: VoiceLoop?
    private var observers: [NSObjectProtocol] = []

    // Formats never change; the audio thread reads them too.
    nonisolated private static let heard = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 1, interleaved: false)!
    nonisolated private static let said = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: Server.speechRate, channels: 1,
        interleaved: false)!

    /// Start opens the microphone and speaker and begins listening. With no chat ID it starts a
    /// new chat; it returns the chat it talks in, or nil when it could not start.
    func start(address: String, chatId: String) async -> String? {
        error = nil
        guard let url = URL(string: address.trimmingCharacters(in: .whitespaces)),
            url.scheme == "https" || url.scheme == "http", url.host() != nil
        else {
            return fail("Enter Covalent's address, such as https://mac.tail1234.ts.net.")
        }
        status = .starting
        guard await AVAudioApplication.requestRecordPermission() else {
            return fail("Covalent can't use the microphone. Allow it in Settings.")
        }
        let server = Server(base: url)
        do {
            guard try await server.voice() else {
                return fail("Covalent's voice is off: start it with OPENAI_API_KEY set.")
            }
            let chat = chatId.isEmpty ? try await server.createChat() : chatId
            try open()
            loop = VoiceLoop(
                .init(
                    transcribe: { try await server.transcribe(wav: wav($0, rate: 16000)) },
                    send: { try await server.send(chatId: chat, text: $0) },
                    speak: { [unowned self] in speak(server.speech($0), onAudio: $1) },
                    // A reply that can't be asked about is treated as finished.
                    pending: { (try? await server.pending($0)) ?? false },
                    cancel: { reply in Task { try? await server.cancel(chatId: reply.chatId) } },
                    onStatus: { [unowned self] in show($0) },
                    onError: { [unowned self] in error = $0.localizedDescription }
                ))
            status = .listening
            return chat
        } catch {
            return fail(error.localizedDescription)
        }
    }

    /// Stop ends voice mode; a reply still being written finishes as text.
    func stop() {
        loop?.close()
        loop = nil
        player.stop()
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        detector = SpeechDetector(rate: 16000)
        level = -100
        status = .off
    }

    private func fail(_ message: String) -> String? {
        error = message
        status = .off
        return nil
    }

    /// Open sets up the audio session and engine: voice chat mode on the loudspeaker, with the
    /// system's echo cancellation so the reply is not heard as the user speaking.
    private func open() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(
            .playAndRecord, mode: .voiceChat, options: [.defaultToSpeaker, .allowBluetoothHFP])
        try session.setActive(true)
        try engine.inputNode.setVoiceProcessingEnabled(true)
        if player.engine == nil { engine.attach(player) }
        engine.connect(player, to: engine.mainMixerNode, format: Self.said)
        try run()
        let center = NotificationCenter.default
        observers = [
            center.addObserver(
                forName: AVAudioSession.interruptionNotification, object: session, queue: .main
            ) { [weak self] note in
                let began = (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt)
                    == AVAudioSession.InterruptionType.began.rawValue
                Task { @MainActor in self?.interrupted(began: began) }
            },
            // The engine stops itself when the audio route or format changes, such as when
            // AirPods connect, and stays stopped until started again.
            center.addObserver(
                forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main
            ) { [weak self] _ in
                Task { @MainActor in self?.reconfigured() }
            },
        ]
    }

    /// Run taps the microphone in its current format and starts the engine and player.
    private func run() throws {
        let input = engine.inputNode
        input.removeTap(onBus: 0)
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, let converter = AVAudioConverter(from: format, to: Self.heard)
        else {
            throw Server.Failure(message: "The microphone's format can't be converted.")
        }
        converter.downmix = true
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
            let frame = Self.convert(buffer, with: converter)
            Task { @MainActor in self?.hear(frame) }
        }
        engine.prepare()
        try engine.start()
        player.play()
    }

    /// Reconfigured restarts the engine after the system stopped it for a route change.
    private func reconfigured() {
        guard status != .off, status != .paused else { return }
        do {
            try run()
        } catch {
            self.error = "Voice mode stopped: \(error.localizedDescription)"
            stop()
        }
    }

    /// Convert turns one microphone buffer into 16 kHz mono samples, on the audio thread.
    private nonisolated static func convert(_ buffer: AVAudioPCMBuffer, with converter: AVAudioConverter)
        -> [Float]
    {
        let ratio = heard.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: heard, frameCapacity: capacity) else { return [] }
        var given = false
        converter.convert(to: out, error: nil) { _, status in
            if given {
                status.pointee = .noDataNow
                return nil
            }
            given = true
            status.pointee = .haveData
            return buffer
        }
        guard let samples = out.floatChannelData?[0] else { return [] }
        return Array(UnsafeBufferPointer(start: samples, count: Int(out.frameLength)))
    }

    private func hear(_ frame: [Float]) {
        guard status != .off, status != .paused, !frame.isEmpty else { return }
        level = SpeechDetector.level(frame)
        for event in detector.feed(frame) {
            switch event {
            case .started:
                loop?.speechStarted()
            case .ended(let utterance):
                Task { await loop?.heard(utterance) }
            }
        }
    }

    private func show(_ status: VoiceStatus) {
        switch status {
        case .listening: self.status = .listening
        case .transcribing: self.status = .transcribing
        case .thinking: self.status = .thinking
        case .speaking: self.status = .speaking
        }
    }

    /// Speak plays a reply's speech as it arrives. Its done returns once the last of it is heard.
    private func speak(
        _ stream: AsyncThrowingStream<[Float], Error>, onAudio: @escaping @MainActor () -> Void
    ) -> Speech {
        let player = self.player
        let task = Task {
            var started = false
            for try await samples in stream {
                try Task.checkCancellation()
                guard let buffer = AVAudioPCMBuffer(
                    pcmFormat: Self.said, frameCapacity: AVAudioFrameCount(samples.count))
                else { continue }
                buffer.frameLength = AVAudioFrameCount(samples.count)
                samples.withUnsafeBufferPointer {
                    buffer.floatChannelData![0].update(from: $0.baseAddress!, count: samples.count)
                }
                // Queued without waiting: the stream keeps arriving while this plays.
                player.scheduleBuffer(buffer, completionHandler: nil)
                if !started { onAudio() }
                started = true
            }
            try Task.checkCancellation()
            // A last, silent buffer reports when everything before it has been played.
            guard let end = AVAudioPCMBuffer(pcmFormat: Self.said, frameCapacity: 240) else { return }
            end.frameLength = 240
            _ = await player.scheduleBuffer(end, completionCallbackType: .dataPlayedBack)
        }
        return Speech(
            stop: {
                task.cancel()
                player.stop()
                player.play()
            },
            done: { try await task.value })
    }

    /// Interrupted pauses voice mode for a call or Siri and resumes it afterwards.
    private func interrupted(began: Bool) {
        guard status != .off else { return }
        if began {
            loop?.pause()
            status = .paused
            return
        }
        do {
            try AVAudioSession.sharedInstance().setActive(true)
            try run()
            status = .listening
        } catch {
            self.error = "Voice mode could not resume: \(error.localizedDescription)"
            stop()
        }
    }
}
