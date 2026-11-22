import SwiftUI

/// VoiceBar is voice mode's controls, shown under the chat while the app's voice mode is on.
struct VoiceBar: View {
    /// height is the room the page leaves for the bar: .voice-native in web/src/styles.css.
    static let height: CGFloat = 76

    let voice: VoiceSession

    var body: some View {
        HStack(spacing: 12) {
            Button(action: tap) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(label).font(.subheadline.weight(.medium))
                    if let error = voice.error {
                        Text(error).font(.footnote).foregroundStyle(.red).lineLimit(1)
                    }
                    // The meter shows the microphone from -60 dB, a quiet room, up to loud speech.
                    ProgressView(value: Double(max(0, min(60, voice.level + 60))) / 60)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Button(action: voice.toggleMute) {
                Image(systemName: voice.muted ? "mic.slash.fill" : "mic.fill")
                    .frame(width: 28, height: 28)
            }
            .accessibilityLabel(voice.muted ? "Unmute" : "Mute")
            .disabled(voice.status == .starting || voice.status == .paused)

            Button(role: .destructive, action: voice.stop) {
                Image(systemName: "xmark").frame(width: 28, height: 28)
            }
            .accessibilityLabel("End voice mode")
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.circle)
        .padding(.horizontal, 16)
        .frame(height: Self.height)
    }

    private var label: String {
        switch voice.status {
        case .off: ""
        case .starting: "Starting the microphone…"
        case .listening: voice.muted ? "Muted" : "Listening"
        case .transcribing: "Transcribing…"
        case .thinking: "Thinking… · tap to stop"
        case .speaking: "Speaking · tap to interrupt"
        case .paused: "Paused · tap to resume"
        }
    }

    private func tap() {
        switch voice.status {
        case .thinking, .speaking: voice.interrupt()
        case .paused: voice.resume()
        default: break
        }
    }
}
