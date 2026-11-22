import Foundation

// A port of web/src/listen.ts, for the protocol of Covalent's /api/listen. The page keeps the
// moment before speech itself; here the speech detector hands it over when a turn starts.

/// Listen streams voice mode's speech to Covalent while it is spoken, so the words of a turn
/// are ready moments after the pause that ends it. Streaming is a shortcut, never a
/// requirement: when Covalent cannot be reached, or the words do not come, commit fails and
/// the caller transcribes the whole recording instead.
@MainActor
public final class Listen {
    private let url: URL
    /// wait bounds how long a turn's words may take.
    private let wait: Duration
    private var socket: URLSessionWebSocketTask?
    private var streaming = false
    private var turns = 0
    private var waiting: [Int: CheckedContinuation<String, Error>] = [:]

    public init(url: URL, wait: Duration = .seconds(5)) {
        self.url = url
        self.wait = wait
        connect()
    }

    /// Start begins streaming a turn with what was heard of it so far. After the connection
    /// has dropped, this opens it again.
    public func start(_ heard: [Float]) {
        if socket == nil { connect() }
        streaming = true
        frame(heard)
    }

    /// Frame sends more of the turn being streamed; between turns it does nothing.
    public func frame(_ samples: [Float]) {
        guard streaming else { return }
        send(.data(pcm16(samples)))
    }

    /// Discard drops the turn being streamed, which will not be finished.
    public func discard() {
        guard streaming else { return }
        streaming = false
        send(.string(#"{"type":"clear"}"#))
    }

    /// Commit ends the turn being streamed and returns its words.
    public func commit() async throws -> String {
        guard streaming, socket != nil else {
            throw Server.Failure(message: "This turn was not streamed.")
        }
        streaming = false
        turns += 1
        let turn = turns
        send(.string(#"{"type":"commit","turn":\#(turn)}"#))
        return try await withCheckedThrowingContinuation { continuation in
            waiting[turn] = continuation
            Task {
                try? await Task.sleep(for: wait)
                settle(turn, nil)
            }
        }
    }

    public func close() {
        let socket = self.socket
        self.socket = nil
        fail()
        socket?.cancel(with: .normalClosure, reason: nil)
    }

    private func connect() {
        let socket = URLSession.shared.webSocketTask(with: url)
        self.socket = socket
        socket.resume()
        Task { [weak self] in
            while let message = try? await socket.receive() {
                guard case .string(let text) = message,
                    let reply = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any]
                else { continue }
                self?.heard(reply)
            }
            self?.dropped(socket)
        }
    }

    /// Heard takes one message from Covalent: a turn's words, or a failure.
    private func heard(_ reply: [String: Any]) {
        // A failure that names no turn may belong to any of them.
        guard let turn = reply["turn"] as? Int else { return fail() }
        settle(turn, reply["text"] as? String)
    }

    private func send(_ message: URLSessionWebSocketTask.Message) {
        guard let socket else { return }
        socket.send(message) { [weak self] error in
            guard error != nil else { return }
            Task { @MainActor in self?.dropped(socket) }
        }
    }

    private func dropped(_ socket: URLSessionWebSocketTask) {
        guard self.socket === socket else { return }
        self.socket = nil
        fail()
    }

    /// Fail gives up on every turn still waiting for its words, and on the one being streamed.
    private func fail() {
        streaming = false
        for turn in Array(waiting.keys) { settle(turn, nil) }
    }

    private func settle(_ turn: Int, _ text: String?) {
        guard let continuation = waiting.removeValue(forKey: turn) else { return }
        guard let text else {
            return continuation.resume(throwing: Server.Failure(message: "The streamed words did not come."))
        }
        continuation.resume(returning: text)
    }
}
