import AVFoundation
import CovalentVoice
import Observation

/// VoiceSession is the proof's voice mode. It listens with echo cancellation, sends each
/// utterance to a Covalent chat as a spoken message, and reads the reply aloud as it is
/// written, until stopped. Because the app records and plays audio, iOS keeps it running with
/// the phone locked or another app in front.
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
    private var server: Server?
    private var chatId = ""
    /// turn changes whenever the user starts over, so work for an older turn stops.
    private var turn = 0
    private var work: Task<Void, Never>?
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
            self.chatId = chatId.isEmpty ? try await server.createChat() : chatId
            self.server = server
            try open()
        } catch {
            return fail(error.localizedDescription)
        }
        status = .listening
        return self.chatId
    }

    /// Stop ends voice mode; a reply still being written finishes as text.
    func stop() {
        turn += 1
        work?.cancel()
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
                // Talking over the assistant stops it, as on the page.
                if status == .thinking || status == .speaking { interrupt() }
            case .ended(let utterance):
                begin(utterance)
            }
        }
    }

    /// Interrupt stops the current turn's speech and its reply, if still being written.
    private func interrupt() {
        turn += 1
        work?.cancel()
        player.stop()
        player.play()
        if let server {
            let chatId = self.chatId
            Task { try? await server.cancel(chatId: chatId) }
        }
        status = .listening
    }

    /// Begin starts a turn from an utterance, replacing any unfinished one.
    private func begin(_ utterance: [Float]) {
        guard let server else { return }
        if status == .thinking || status == .speaking { interrupt() }
        turn += 1
        let turn = self.turn
        let chatId = self.chatId
        work?.cancel()
        work = Task {
            do {
                status = .transcribing
                let text = try await server.transcribe(wav: wav(utterance, rate: 16000))
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                guard turn == self.turn else { return }
                guard !text.isEmpty else {
                    status = .listening
                    return
                }
                status = .thinking
                let reply = try await server.send(chatId: chatId, text: text)
                guard turn == self.turn else { return }
                try await speak(server.speech(reply), turn: turn)
                guard turn == self.turn else { return }
                status = .listening
            } catch {
                guard turn == self.turn, !(error is CancellationError) else { return }
                self.error = error.localizedDescription
                status = .listening
            }
        }
    }

    /// Speak plays the speech stream as it arrives and returns once the last of it is heard.
    private func speak(_ speech: AsyncThrowingStream<[Float], Error>, turn: Int) async throws {
        for try await samples in speech {
            guard turn == self.turn else { return }
            guard let buffer = AVAudioPCMBuffer(
                pcmFormat: Self.said, frameCapacity: AVAudioFrameCount(samples.count))
            else { continue }
            buffer.frameLength = AVAudioFrameCount(samples.count)
            samples.withUnsafeBufferPointer {
                buffer.floatChannelData![0].update(from: $0.baseAddress!, count: samples.count)
            }
            // Queued without waiting: the stream keeps arriving while this plays.
            player.scheduleBuffer(buffer, completionHandler: nil)
            if status == .thinking { status = .speaking }
        }
        // A last, silent buffer reports when everything before it has been played.
        guard let end = AVAudioPCMBuffer(pcmFormat: Self.said, frameCapacity: 240) else { return }
        end.frameLength = 240
        _ = await player.scheduleBuffer(end, completionCallbackType: .dataPlayedBack)
    }

    /// Interrupted pauses voice mode for a call or Siri and resumes it afterwards.
    private func interrupted(began: Bool) {
        guard status != .off else { return }
        if began {
            if status == .thinking || status == .speaking { interrupt() }
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
