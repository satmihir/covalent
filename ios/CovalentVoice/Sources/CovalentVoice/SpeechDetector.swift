import Foundation

/// SpeechDetector finds utterances in the microphone's samples by their loudness. Speech is
/// sound a margin above the room's noise floor that lasts a moment; a long enough quiet ends
/// the utterance. These are the page's rules with loudness in place of its Silero model.
public struct SpeechDetector {
    public enum Event: Equatable {
        /// Started means the user began to speak, which interrupts a reply.
        case started
        /// Ended carries one utterance, from just before it started to its end.
        case ended([Float])
    }

    /// rate is the sample rate of what `feed` is given.
    public let rate: Double
    /// minimumSpeech is how long sound must last to count as speech, in seconds.
    public var minimumSpeech = 0.25
    /// pause is how long a quiet must last to end an utterance; shorter ones are a breath.
    public var pause = 0.8
    /// margin is how many decibels above the noise floor count as speech.
    public var margin: Float = 15
    /// quietest is the threshold in a silent room, so hiss never counts as speech.
    public var quietest: Float = -45
    /// longest ends an utterance that never pauses, in seconds.
    public var longest = 30.0

    /// floor is the room's noise level, starting from the first frame heard.
    private var floor: Float?
    private var speaking = false
    private var loud = 0.0
    private var quiet = 0.0
    /// recent keeps the moment before speech is confirmed, so its first syllable is not lost.
    private var recent: [Float] = []
    private var utterance: [Float] = []

    public init(rate: Double) {
        self.rate = rate
    }

    /// Level is a frame's loudness in decibels relative to full scale.
    public static func level(_ samples: [Float]) -> Float {
        guard !samples.isEmpty else { return -100 }
        let power = samples.reduce(0) { $0 + $1 * $1 } / Float(samples.count)
        return max(-100, 10 * log10(max(power, 1e-10)))
    }

    /// Feed takes the next frame of samples, ideally 20 to 100 ms, and reports what changed.
    public mutating func feed(_ samples: [Float]) -> [Event] {
        let duration = Double(samples.count) / rate
        let level = Self.level(samples)
        let floor = self.floor ?? level
        let threshold = max(floor + margin, quietest)
        var events: [Event] = []
        if speaking {
            utterance += samples
            quiet = level > threshold ? 0 : quiet + duration
            if quiet >= pause || Double(utterance.count) / rate >= longest {
                events.append(.ended(utterance))
                speaking = false
                utterance = []
                loud = 0
                quiet = 0
            }
            return events
        }
        recent += samples
        // Keep the last half second, enough to hold the start of speech while it is confirmed.
        let keep = Int(rate * 0.5)
        if recent.count > keep { recent.removeFirst(recent.count - keep) }
        if level > threshold {
            loud += duration
            if loud >= minimumSpeech {
                speaking = true
                utterance = recent
                recent = []
                events.append(.started)
            }
        } else {
            loud = 0
            // The floor follows a quieter room at once and a louder one over a few seconds.
            self.floor = level < floor ? level : floor + (level - floor) * Float(min(1, duration / 3))
        }
        return events
    }
}
