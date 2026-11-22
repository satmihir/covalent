import Foundation

// A port of web/src/voiceLoop.ts. The rules live in both files: change one, change the other.
// The tests carry the same scenario names, so a missing one shows.

/// VoiceStatus is what voice mode is doing in a turn.
public enum VoiceStatus: Sendable {
    case listening, transcribing, thinking, speaking
}

/// Speech is a reply being read aloud: stop silences it, and done returns once it has all been
/// heard or it was stopped.
public struct Speech {
    public let stop: @MainActor () -> Void
    public let done: @MainActor () async throws -> Void

    public init(stop: @escaping @MainActor () -> Void, done: @escaping @MainActor () async throws -> Void) {
        self.stop = stop
        self.done = done
    }
}

/// VoiceLoop holds voice mode's turn-taking rules apart from the microphone, network and
/// speaker, which it is given, so the rules can be tested on their own.
@MainActor
public final class VoiceLoop {
    /// Deps is the world the loop acts on.
    public struct Deps {
        public var transcribe: @MainActor ([Float]) async throws -> String
        /// send posts a spoken message and says where its reply will appear.
        public var send: @MainActor (String) async throws -> Server.Reply
        /// speak reads a reply aloud as it is written; onAudio runs when the first sound is due.
        public var speak: @MainActor (Server.Reply, _ onAudio: @escaping @MainActor () -> Void) -> Speech
        /// pending reports whether a reply is still being written.
        public var pending: @MainActor (Server.Reply) async -> Bool
        /// cancel stops the chat's reply that is still being written.
        public var cancel: @MainActor (Server.Reply) -> Void
        public var onStatus: @MainActor (VoiceStatus) -> Void
        public var onError: @MainActor (Error) -> Void

        public init(
            transcribe: @escaping @MainActor ([Float]) async throws -> String,
            send: @escaping @MainActor (String) async throws -> Server.Reply,
            speak: @escaping @MainActor (Server.Reply, _ onAudio: @escaping @MainActor () -> Void) -> Speech,
            pending: @escaping @MainActor (Server.Reply) async -> Bool,
            cancel: @escaping @MainActor (Server.Reply) -> Void,
            onStatus: @escaping @MainActor (VoiceStatus) -> Void,
            onError: @escaping @MainActor (Error) -> Void
        ) {
            self.transcribe = transcribe
            self.send = send
            self.speak = speak
            self.pending = pending
            self.cancel = cancel
            self.onStatus = onStatus
            self.onError = onError
        }
    }

    /// retries bounds how often one reply is asked for again after its speech stream ends
    /// early, which happens when the model starts the reply over.
    private static let retries = 5

    public private(set) var status = VoiceStatus.listening
    /// turn changes whenever the user starts over, so late results of an older turn are ignored.
    private var turn = 0
    private var speech: Speech?
    private var reply: Server.Reply?
    private let deps: Deps

    public init(_ deps: Deps) {
        self.deps = deps
    }

    /// Heard takes one finished utterance; it starts a new turn, replacing any unfinished one.
    public func heard(_ audio: [Float]) async {
        let turn = interrupt()
        set(.transcribing)
        do {
            let text = try await deps.transcribe(audio).trimmingCharacters(in: .whitespacesAndNewlines)
            guard turn == self.turn else { return }
            guard !text.isEmpty else { return set(.listening) }
            set(.thinking)
            let reply = try await deps.send(text)
            guard turn == self.turn else {
                // The user spoke again while this message was on its way; its reply is not wanted.
                return cancelIfPending(reply)
            }
            self.reply = reply
            try await play(turn, reply)
        } catch {
            fail(turn, error)
        }
    }

    /// SpeechStarted is the user talking over the assistant, which stops it.
    public func speechStarted() {
        if status == .thinking || status == .speaking { interrupt() }
    }

    /// Interrupt stops the current turn's reply, cancelling it if still being written.
    @discardableResult
    public func interrupt() -> Int {
        let reply = self.reply
        close()
        if let reply { cancelIfPending(reply) }
        set(.listening)
        return turn
    }

    /// Close stops the speech and ignores this loop's late results, and lets a reply still being
    /// written finish as text.
    public func close() {
        turn += 1
        speech?.stop()
        speech = nil
        reply = nil
    }

    /// Pause stops the speech and ignores late results, as close does, but the loop stays open:
    /// the next utterance starts a turn as usual. A reply still being written finishes as text.
    public func pause() {
        close()
        set(.listening)
    }

    /// Play reads a reply aloud, asking again while its stream ends with the reply unfinished.
    private func play(_ turn: Int, _ reply: Server.Reply) async throws {
        for _ in 0...Self.retries {
            let speech = deps.speak(reply) { [weak self] in
                if turn == self?.turn { self?.set(.speaking) }
            }
            self.speech = speech
            try await speech.done()
            guard turn == self.turn else { return }
            let pending = await deps.pending(reply)
            guard turn == self.turn else { return }
            if !pending { break }
        }
        speech = nil
        self.reply = nil
        set(.listening)
    }

    private func cancelIfPending(_ reply: Server.Reply) {
        Task {
            if await deps.pending(reply) { deps.cancel(reply) }
        }
    }

    private func fail(_ turn: Int, _ error: Error) {
        guard turn == self.turn else { return }
        deps.onError(error)
        speech = nil
        reply = nil
        set(.listening)
    }

    private func set(_ status: VoiceStatus) {
        guard self.status != status else { return }
        self.status = status
        deps.onStatus(status)
    }
}
