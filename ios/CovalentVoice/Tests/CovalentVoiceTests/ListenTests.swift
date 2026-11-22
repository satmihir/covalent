import CovalentVoice
import Foundation
import Network
import Testing

/// FakeListen is Covalent's /api/listen on a loopback port: it records what a client sends and
/// lets the test answer or drop the connection.
private final class FakeListen: @unchecked Sendable {
    enum Message: Equatable {
        case speech(Data)
        case text(String)
    }

    private let listener: NWListener
    private let queue = DispatchQueue(label: "fake-listen")
    private var connection: NWConnection?
    private var messages: AsyncStream<Message>.Iterator
    private let heard: AsyncStream<Message>.Continuation

    var url: URL {
        URL(string: "ws://127.0.0.1:\(listener.port!.rawValue)/api/listen")!
    }

    init() async throws {
        let parameters = NWParameters.tcp
        parameters.defaultProtocolStack.applicationProtocols.insert(NWProtocolWebSocket.Options(), at: 0)
        parameters.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: .any)
        let listener = try NWListener(using: parameters)
        let (stream, heard) = AsyncStream<Message>.makeStream()
        self.listener = listener
        self.heard = heard
        messages = stream.makeAsyncIterator()
        // A listener started with no connection handler fails, so the handler comes first.
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { return }
            self.connection = connection
            connection.start(queue: queue)
            receive(on: connection)
        }
        try await withCheckedThrowingContinuation { (ready: CheckedContinuation<Void, Error>) in
            listener.stateUpdateHandler = { state in
                switch state {
                case .ready: ready.resume()
                case .failed(let error): ready.resume(throwing: error)
                default: return
                }
                listener.stateUpdateHandler = nil
            }
            listener.start(queue: queue)
        }
    }

    deinit {
        listener.cancel()
        connection?.cancel()
    }

    /// Next waits for the next thing a client sends.
    func next() async -> Message? {
        await messages.next()
    }

    func reply(_ text: String) {
        let metadata = NWProtocolWebSocket.Metadata(opcode: .text)
        let context = NWConnection.ContentContext(identifier: "reply", metadata: [metadata])
        connection?.send(content: Data(text.utf8), contentContext: context, completion: .idempotent)
    }

    func drop() {
        connection?.cancel()
    }

    private func receive(on connection: NWConnection) {
        connection.receiveMessage { [weak self] data, context, _, error in
            guard let self, error == nil else { return }
            let metadata = context?.protocolMetadata(definition: NWProtocolWebSocket.definition)
            switch (metadata as? NWProtocolWebSocket.Metadata)?.opcode {
            case .binary: heard.yield(.speech(data ?? Data()))
            case .text: heard.yield(.text(String(decoding: data ?? Data(), as: UTF8.self)))
            default: break
            }
            receive(on: connection)
        }
    }
}

@MainActor
struct ListenTests {
    @Test func aStreamedTurnSendsItsSpeechThenACommitAndHearsItsWords() async throws {
        let server = try await FakeListen()
        let listen = Listen(url: server.url)
        listen.start([0, 0.5])
        listen.frame([-1])
        async let words = listen.commit()
        #expect(await server.next() == .speech(pcm16([0, 0.5])))
        #expect(await server.next() == .speech(pcm16([-1])))
        #expect(await server.next() == .text(#"{"type":"commit","turn":1}"#))
        server.reply(#"{"turn":1,"text":"hello there"}"#)
        #expect(try await words == "hello there")
    }

    @Test func soundBetweenTurnsIsNotSent() async throws {
        let server = try await FakeListen()
        let listen = Listen(url: server.url)
        listen.frame([0.25])
        listen.start([0.5])
        #expect(await server.next() == .speech(pcm16([0.5])))
    }

    @Test(arguments: [#"{"turn":1,"error":"no speech"}"#, #"{"error":"the session failed"}"#])
    func aTurnCovalentCouldNotTranscribeFailsItsCommit(reply: String) async throws {
        let server = try await FakeListen()
        let listen = Listen(url: server.url)
        listen.start([0.5])
        let words = Task { try await listen.commit() }
        #expect(await server.next() == .speech(pcm16([0.5])))
        _ = await server.next()
        server.reply(reply)
        await #expect(throws: Server.Failure.self) { try await words.value }
    }

    @Test func wordsThatDoNotComeInTimeFailTheCommit() async throws {
        let server = try await FakeListen()
        let listen = Listen(url: server.url, wait: .milliseconds(100))
        listen.start([0.5])
        await #expect(throws: Server.Failure.self) { try await listen.commit() }
    }

    @Test func aDiscardedTurnIsClearedAndCannotBeCommitted() async throws {
        let server = try await FakeListen()
        let listen = Listen(url: server.url)
        listen.start([0.5])
        listen.discard()
        #expect(await server.next() == .speech(pcm16([0.5])))
        #expect(await server.next() == .text(#"{"type":"clear"}"#))
        await #expect(throws: Server.Failure.self) { try await listen.commit() }
    }

    @Test func withCovalentUnreachableTheCommitFailsLongBeforeTheWait() async throws {
        // Nothing listens on the discard port, so the connection is refused.
        let listen = Listen(url: URL(string: "ws://127.0.0.1:9/api/listen")!, wait: .seconds(60))
        listen.start([0.5])
        let clock = ContinuousClock()
        let began = clock.now
        await #expect(throws: Server.Failure.self) { try await listen.commit() }
        #expect(clock.now - began < .seconds(10))
    }

    @Test func afterTheConnectionDropsTheNextTurnOpensItAgain() async throws {
        let server = try await FakeListen()
        let listen = Listen(url: server.url, wait: .seconds(60))
        listen.start([0.5])
        #expect(await server.next() == .speech(pcm16([0.5])))
        server.drop()
        // The turn in hand fails once the drop is noticed.
        await #expect(throws: Server.Failure.self) { try await listen.commit() }

        listen.start([0.25])
        async let words = listen.commit()
        #expect(await server.next() == .speech(pcm16([0.25])))
        // Whether the failed turn was counted depends on when the drop was noticed.
        guard case .text(let commit) = await server.next(),
            let turn = (try JSONSerialization.jsonObject(with: Data(commit.utf8)) as? [String: Any])?["turn"]
        else {
            Issue.record("expected a commit")
            return
        }
        server.reply(#"{"turn":\#(turn),"text":"again"}"#)
        #expect(try await words == "again")
    }
}
