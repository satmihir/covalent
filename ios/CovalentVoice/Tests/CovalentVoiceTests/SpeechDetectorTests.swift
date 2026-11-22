import CovalentVoice
import Foundation
import Testing

/// Frames builds 50 ms frames at 16 kHz: a sine at the given loudness, or silence-level noise.
private func frames(_ count: Int, decibels: Float) -> [[Float]] {
    let amplitude = pow(10, decibels / 20) * Float(2).squareRoot()
    return (0..<count).map { frame in
        (0..<800).map { i in amplitude * sin(Float(frame * 800 + i) * 0.2) }
    }
}

private func run(_ detector: inout SpeechDetector, _ frames: [[Float]]) -> [SpeechDetector.Event] {
    frames.flatMap { detector.feed($0) }
}

@Test func speechAboveTheRoomStartsAnUtteranceAndAPauseEndsIt() {
    var detector = SpeechDetector(rate: 16000)
    #expect(run(&detector, frames(20, decibels: -60)).isEmpty)
    let started = run(&detector, frames(4, decibels: -60) + frames(6, decibels: -20))
    #expect(started == [.started])
    // What was heard so far is there to be streamed: the half second kept up to the frame
    // that confirmed the speech, and the one frame since.
    #expect(detector.utterance.count == Int(16000 * 0.55))
    // A short breath does not end it; 800 ms of quiet does.
    #expect(run(&detector, frames(6, decibels: -60) + frames(4, decibels: -20)).isEmpty)
    let ended = run(&detector, frames(16, decibels: -60))
    guard case .ended(let utterance) = ended.first, ended.count == 1 else {
        Issue.record("expected one utterance, got \(ended)")
        return
    }
    // The half second held while speech was confirmed, then the last loud frame, the breath,
    // more speech and the closing pause: 0.5 + 0.05 + 0.3 + 0.2 + 0.8 seconds.
    #expect(utterance.count == Int(16000 * 1.85))
}

@Test func aClickTooShortForSpeechStartsNothing() {
    var detector = SpeechDetector(rate: 16000)
    let events = run(&detector, frames(10, decibels: -60) + frames(2, decibels: -15) + frames(20, decibels: -60))
    #expect(events.isEmpty)
}

@Test func aSteadilyNoisyRoomRaisesTheBarForSpeech() {
    var detector = SpeechDetector(rate: 16000)
    // A fan at -38 dB, from the first frame on, is never speech.
    #expect(run(&detector, frames(100, decibels: -38)).isEmpty)
    // Talking over it is.
    #expect(run(&detector, frames(8, decibels: -15)) == [.started])
}

@Test func anUtteranceThatNeverPausesEndsAtTheLongestLength() {
    var detector = SpeechDetector(rate: 16000)
    detector.longest = 2
    let events = run(&detector, frames(10, decibels: -60) + frames(60, decibels: -20))
    #expect(events.first == .started)
    #expect(events.filter { if case .ended = $0 { true } else { false } }.count == 1)
}
