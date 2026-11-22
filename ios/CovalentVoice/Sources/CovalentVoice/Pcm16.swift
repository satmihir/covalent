import Foundation

/// Pcm16 encodes mono samples between -1 and 1 as 16-bit little-endian PCM.
public func pcm16(_ samples: [Float]) -> Data {
    var data = Data(capacity: samples.count * 2)
    for sample in samples {
        let clamped = max(-1, min(1, sample))
        let value = Int16(clamped < 0 ? clamped * 32768 : clamped * 32767)
        withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
    }
    return data
}

/// Wav encodes mono samples between -1 and 1 as a 16-bit WAV file, the form Covalent's
/// transcription route accepts, as the page's own `wav` does.
public func wav(_ samples: [Float], rate: Int) -> Data {
    var data = Data()
    func append<T: FixedWidthInteger>(_ value: T) {
        withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
    }
    let bytes = samples.count * 2
    data.append(contentsOf: Array("RIFF".utf8))
    append(UInt32(36 + bytes))
    data.append(contentsOf: Array("WAVEfmt ".utf8))
    append(UInt32(16))
    append(UInt16(1))  // PCM
    append(UInt16(1))  // mono
    append(UInt32(rate))
    append(UInt32(rate * 2))
    append(UInt16(2))
    append(UInt16(16))
    data.append(contentsOf: Array("data".utf8))
    append(UInt32(bytes))
    return data + pcm16(samples)
}

/// Pcm16Decoder turns Covalent's speech stream, 16-bit little-endian mono, into samples. A
/// chunk of the stream may end halfway through a sample; that byte waits for the next chunk.
public struct Pcm16Decoder {
    private var carry: UInt8?

    public init() {}

    /// Decode returns the whole samples in `bytes`, keeping a split one for later.
    public mutating func decode(_ bytes: [UInt8]) -> [Float] {
        var bytes = bytes
        if let carry {
            bytes.insert(carry, at: 0)
            self.carry = nil
        }
        if bytes.count % 2 == 1 {
            carry = bytes.removeLast()
        }
        return stride(from: 0, to: bytes.count, by: 2).map { i in
            Float(Int16(bitPattern: UInt16(bytes[i]) | UInt16(bytes[i + 1]) << 8)) / 32768
        }
    }
}
