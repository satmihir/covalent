import Foundation

/// Server calls the Covalent routes voice mode uses, the same ones the page's voice mode does.
public struct Server: Sendable {
    /// Reply names a spoken message's reply, which is read aloud as it is written.
    public struct Reply: Equatable, Sendable {
        public let chatId: String
        public let replyId: String

        public init(chatId: String, replyId: String) {
            self.chatId = chatId
            self.replyId = replyId
        }
    }

    /// Failure is a refused request, in the words Covalent gave.
    public struct Failure: LocalizedError {
        public let message: String
        public var errorDescription: String? { message }

        public init(message: String) {
            self.message = message
        }
    }

    public let base: URL

    public init(base: URL) {
        self.base = base
    }

    /// Voice reports whether Covalent can transcribe and speak, which needs its OpenAI key.
    public func voice() async throws -> Bool {
        let body = try await json("GET", "api/voice")
        return body["available"] as? Bool ?? false
    }

    /// CreateChat starts a chat with Covalent's default model and returns its ID.
    public func createChat() async throws -> String {
        let body = try await json("POST", "api/chats", body: Data("{}".utf8))
        guard let chat = body["chat"] as? [String: Any], let id = chat["id"] as? String else {
            throw Failure(message: "Covalent did not return the new chat.")
        }
        return id
    }

    /// Transcribe turns a recorded utterance into text.
    public func transcribe(wav: Data) async throws -> String {
        let body = try await json("POST", "api/transcribe", body: wav, type: "audio/wav")
        return body["text"] as? String ?? ""
    }

    /// Send posts a spoken message and returns its reply, which is still being written.
    public func send(chatId: String, text: String) async throws -> Reply {
        let message: [String: Any] = ["request_id": UUID().uuidString, "text": text, "spoken": true]
        let body = try await json(
            "POST", "api/chats/\(chatId)/messages",
            body: try JSONSerialization.data(withJSONObject: message))
        guard let messages = body["messages"] as? [[String: Any]], let last = messages.last,
            last["role"] as? String == "assistant", let id = last["id"] as? String
        else {
            throw Failure(message: "Covalent did not return the reply.")
        }
        return Reply(chatId: chatId, replyId: id)
    }

    /// Speech streams a reply read aloud: 16-bit mono samples at `speechRate`, in chunks as
    /// they arrive. The stream ends when the reading ends; cancelling it stops the reading.
    public func speech(_ reply: Reply) -> AsyncThrowingStream<[Float], Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let path = "api/chats/\(reply.chatId)/messages/\(reply.replyId)/speech"
                    let (bytes, response) = try await URLSession.shared.bytes(
                        for: request("GET", path))
                    try await refused(response, bytes: bytes)
                    var decoder = Pcm16Decoder()
                    var chunk: [UInt8] = []
                    // 50 ms of speech per chunk keeps the first words quick and the scheduling light.
                    chunk.reserveCapacity(2400)
                    for try await byte in bytes {
                        chunk.append(byte)
                        if chunk.count >= 2400 {
                            continuation.yield(decoder.decode(chunk))
                            chunk.removeAll(keepingCapacity: true)
                        }
                    }
                    let rest = decoder.decode(chunk)
                    if !rest.isEmpty { continuation.yield(rest) }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// Pending reports whether a reply is still being written.
    public func pending(_ reply: Reply) async throws -> Bool {
        let body = try await json("GET", "api/chats/\(reply.chatId)")
        let messages = body["messages"] as? [[String: Any]] ?? []
        return messages.contains {
            $0["id"] as? String == reply.replyId && $0["status"] as? String == "pending"
        }
    }

    /// Cancel stops the chat's reply that is still being written.
    public func cancel(chatId: String) async throws {
        _ = try await json("POST", "api/chats/\(chatId)/cancel")
    }

    /// speechRate is the sample rate of the speech stream.
    public static let speechRate = 24000.0

    private func request(_ method: String, _ path: String, body: Data? = nil, type: String? = nil)
        -> URLRequest
    {
        var request = URLRequest(url: base.appending(path: path))
        request.httpMethod = method
        request.httpBody = body
        if let type = type ?? (body == nil ? nil : "application/json") {
            request.setValue(type, forHTTPHeaderField: "Content-Type")
        }
        return request
    }

    private func json(_ method: String, _ path: String, body: Data? = nil, type: String? = nil)
        async throws -> [String: Any]
    {
        let (data, response) = try await URLSession.shared.data(
            for: request(method, path, body: body, type: type))
        let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        if let status = (response as? HTTPURLResponse)?.statusCode, !(200..<300).contains(status) {
            throw Failure(message: object["message"] as? String ?? "Covalent answered \(status).")
        }
        return object
    }

    /// Refused turns a failed speech request into its message; a good one is left to stream.
    private func refused(_ response: URLResponse, bytes: URLSession.AsyncBytes) async throws {
        guard let status = (response as? HTTPURLResponse)?.statusCode, !(200..<300).contains(status)
        else { return }
        var data = Data()
        for try await byte in bytes { data.append(byte) }
        let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        throw Failure(message: object?["message"] as? String ?? "Covalent answered \(status).")
    }
}
