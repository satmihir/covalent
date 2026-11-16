import CovalentVoice
import Foundation
import Testing

@Test func aWavFileHasTheHeaderTheTranscriptionRouteExpects() {
    let data = wav([0, 1, -1, 0.5], rate: 16000)
    let bytes = [UInt8](data)
    #expect(bytes.count == 44 + 8)
    #expect(String(decoding: bytes[0..<4], as: UTF8.self) == "RIFF")
    #expect(String(decoding: bytes[8..<16], as: UTF8.self) == "WAVEfmt ")
    let word = { (at: Int) in UInt32(bytes[at]) | UInt32(bytes[at + 1]) << 8 | UInt32(bytes[at + 2]) << 16 | UInt32(bytes[at + 3]) << 24 }
    #expect(word(24) == 16000)
    #expect(word(40) == 8)
    let sample = { (i: Int) in Int16(bitPattern: UInt16(bytes[44 + i * 2]) | UInt16(bytes[45 + i * 2]) << 8) }
    #expect([sample(0), sample(1), sample(2), sample(3)] == [0, 32767, -32768, 16383])
}

@Test func speechSplitHalfwayThroughASampleIsJoinedAcrossChunks() {
    // Samples 0, 16384 (0.5) and -32768 (-1), little-endian, cut after an odd byte.
    let stream: [UInt8] = [0x00, 0x00, 0x00, 0x40, 0x00, 0x80]
    var decoder = Pcm16Decoder()
    #expect(decoder.decode(Array(stream[0..<3])) == [0])
    #expect(decoder.decode(Array(stream[3..<6])) == [0.5, -1])
    #expect(decoder.decode([]) == [])
}
