import CovalentVoice
import Testing

// The scenarios of web/src/voiceLoop.test.ts, under the same names.

/// Deferred is a result the test settles by hand.
@MainActor
private final class Deferred<T> {
    private var result: Result<T, Error>?
    private var waiter: UnsafeContinuation<T, Error>?

    func value() async throws -> T {
        if let result { return try result.get() }
        return try await withUnsafeThrowingContinuation { waiter = $0 }
    }

    func settle(_ result: Result<T, Error>) {
        self.result = result
        waiter?.resume(with: result)
        waiter = nil
    }
}

private struct Offline: Error {}

/// Fake records what the loop asks of the world and lets the test answer each request.
@MainActor
private final class Fake {
    @MainActor final class Spoken {
        let reply: Server.Reply
        let onAudio: @MainActor () -> Void
        let done = Deferred<Void>()
        var stopped = false

        init(reply: Server.Reply, onAudio: @escaping @MainActor () -> Void) {
            self.reply = reply
            self.onAudio = onAudio
        }
    }

    var statuses: [VoiceStatus] = []
    var transcripts: [Deferred<String>] = []
    var sends: [(text: String, reply: Deferred<Server.Reply>)] = []
    var speeches: [Spoken] = []
    var pending: Set<String> = []
    var cancelled: [String] = []
    var errors: [Error] = []

    lazy var loop = VoiceLoop(
        .init(
            transcribe: { _ in
                let words = Deferred<String>()
                self.transcripts.append(words)
                return try await words.value()
            },
            send: { text in
                let reply = Deferred<Server.Reply>()
                self.sends.append((text, reply))
                return try await reply.value()
            },
            speak: { reply, onAudio in
                let speech = Spoken(reply: reply, onAudio: onAudio)
                self.speeches.append(speech)
                return Speech(stop: { speech.stopped = true }, done: { try await speech.done.value() })
            },
            pending: { self.pending.contains($0.replyId) },
            cancel: { self.cancelled.append($0.chatId) },
            onStatus: { self.statuses.append($0) },
            onError: { self.errors.append($0) }
        ))

    /// Hear gives the loop one utterance and lets it get as far as asking for the words.
    func hear() async {
        Task { await loop.heard([Float](repeating: 0, count: 16)) }
        await settle()
    }

    /// Ask runs one turn up to a reply being spoken, and returns that reply.
    func ask(_ text: String, _ replyId: String) async -> Server.Reply {
        await hear()
        transcripts.last!.settle(.success(text))
        await settle()
        let reply = Server.Reply(chatId: "chat", replyId: replyId)
        pending.insert(replyId)
        sends.last!.reply.settle(.success(reply))
        await settle()
        return reply
    }
}

/// Settle lets the loop react to what the test just did. Everything here runs on the main
/// actor, whose queue is first in, first out, so each yield lets the loop take a step.
@MainActor
private func settle() async {
    for _ in 0..<20 { await Task.yield() }
}

@MainActor
struct VoiceLoopTests {
    @Test("a turn is heard, sent, read aloud as it arrives, and then the loop listens again")
    func aTurn() async {
        let f = Fake()
        let reply = await f.ask("what rises?", "r1")
        #expect(f.sends.map(\.text) == ["what rises?"])
        #expect(f.speeches.map(\.reply) == [reply])
        #expect(f.loop.status == .thinking)
        f.speeches[0].onAudio()
        #expect(f.loop.status == .speaking)
        f.pending.remove("r1")
        f.speeches[0].done.settle(.success(()))
        await settle()
        #expect(f.statuses == [.transcribing, .thinking, .speaking, .listening])
        #expect(f.cancelled.isEmpty)
    }

    @Test("silence that transcribes to nothing sends nothing")
    func silence() async {
        let f = Fake()
        await f.hear()
        f.transcripts[0].settle(.success("  "))
        await settle()
        #expect(f.sends.isEmpty)
        #expect(f.loop.status == .listening)
    }

    @Test("talking over a reply still being written stops it and cancels it")
    func talkingOverAPendingReply() async {
        let f = Fake()
        _ = await f.ask("tell me everything", "r1")
        f.speeches[0].onAudio()
        f.loop.speechStarted()
        await settle()
        #expect(f.speeches[0].stopped)
        #expect(f.cancelled == ["chat"])
        #expect(f.loop.status == .listening)
        // The stopped speech finishing later changes nothing.
        f.speeches[0].done.settle(.success(()))
        await settle()
        #expect(f.speeches.count == 1)
        _ = await f.ask("actually, just one thing", "r2")
        #expect(f.speeches.last?.reply.replyId == "r2")
    }

    @Test("talking over a reply that is finished only stops the speech")
    func talkingOverAFinishedReply() async {
        let f = Fake()
        _ = await f.ask("hi", "r1")
        f.speeches[0].onAudio()
        f.pending.remove("r1")
        f.loop.speechStarted()
        await settle()
        #expect(f.speeches[0].stopped)
        #expect(f.cancelled.isEmpty)
    }

    @Test("starting to talk while the last words are transcribed does not drop them")
    func talkingWhileTranscribing() async {
        let f = Fake()
        await f.hear()
        f.loop.speechStarted()
        f.transcripts[0].settle(.success("first words"))
        await settle()
        #expect(f.sends.map(\.text) == ["first words"])
    }

    @Test("a message overtaken by a newer utterance has its reply cancelled and never read")
    func overtaken() async {
        let f = Fake()
        await f.hear()
        f.transcripts[0].settle(.success("first"))
        await settle()
        await f.hear()
        f.pending.insert("r1")
        f.sends[0].reply.settle(.success(Server.Reply(chatId: "chat", replyId: "r1")))
        await settle()
        #expect(f.cancelled == ["chat"])
        #expect(f.speeches.isEmpty)
        #expect(f.loop.status == .transcribing)
    }

    @Test("a speech stream that ends while the reply is rewritten is asked for again, a few times")
    func rewrittenReply() async {
        let f = Fake()
        _ = await f.ask("hi", "r1")
        for _ in 0..<7 {
            f.speeches.last!.done.settle(.success(()))
            await settle()
        }
        #expect(f.speeches.count == 6, "the first request and five more")
        #expect(f.loop.status == .listening)
    }

    @Test("ending voice mode stops the speech and lets the reply finish as text")
    func ending() async {
        let f = Fake()
        _ = await f.ask("hi", "r1")
        f.speeches[0].onAudio()
        f.loop.close()
        #expect(f.speeches[0].stopped)
        f.speeches[0].done.settle(.success(()))
        await settle()
        #expect(f.cancelled.isEmpty)
        #expect(f.speeches.count == 1)
    }

    @Test("pausing stops the speech, lets the reply finish as text, and the next turn works")
    func pausing() async {
        let f = Fake()
        _ = await f.ask("hi", "r1")
        f.speeches[0].onAudio()
        f.loop.pause()
        #expect(f.speeches[0].stopped)
        #expect(f.loop.status == .listening)
        // The paused reply's speech ending late starts nothing.
        f.speeches[0].done.settle(.success(()))
        await settle()
        #expect(f.cancelled.isEmpty)
        #expect(f.speeches.count == 1)
        _ = await f.ask("again", "r2")
        #expect(f.speeches.count == 2)
        #expect(f.loop.status == .thinking)
    }

    // The page's send reports its own failures; here a failed send is reported like any other.
    @Test("a failure is reported and the loop listens again")
    func failure() async {
        let f = Fake()
        await f.hear()
        f.transcripts[0].settle(.failure(Offline()))
        await settle()
        #expect(f.errors.count == 1 && f.errors[0] is Offline)
        #expect(f.loop.status == .listening)
        await f.hear()
        f.transcripts[1].settle(.success("hello"))
        await settle()
        f.sends[0].reply.settle(.failure(Offline()))
        await settle()
        #expect(f.errors.count == 2)
        #expect(f.loop.status == .listening)
    }
}
