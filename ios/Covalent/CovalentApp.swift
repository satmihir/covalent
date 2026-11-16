import SwiftUI

@main
struct CovalentApp: App {
    var body: some Scene {
        WindowGroup { ProofView() }
    }
}

/// ProofView is step 14's one screen: where Covalent is, which chat to talk in, and a button.
/// Its only job is to show that voice mode keeps going in the background on a real phone.
struct ProofView: View {
    @AppStorage("address") private var address = ""
    @AppStorage("chat") private var chatId = ""
    @State private var voice = VoiceSession()

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("https://mac.tail1234.ts.net", text: $address)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Chat ID, or empty for a new chat", text: $chatId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                } header: {
                    Text("Covalent")
                } footer: {
                    Text("A new chat appears in Covalent's sidebar after a reload.")
                }
                .disabled(voice.status != .off)

                Section {
                    Button(voice.status == .off ? "Start talking" : "Stop", role: voice.status == .off ? nil : .destructive) {
                        if voice.status == .off {
                            Task {
                                if let chat = await voice.start(address: address, chatId: chatId) {
                                    chatId = chat
                                }
                            }
                        } else {
                            voice.stop()
                        }
                    }
                    .disabled(voice.status == .starting)
                }

                Section("Status") {
                    Text(voice.status.rawValue)
                    // The meter shows the microphone from -60 dB, a quiet room, up to loud speech.
                    ProgressView(value: Double(max(0, min(60, voice.level + 60))) / 60)
                    if let error = voice.error {
                        Text(error).foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Covalent voice")
        }
    }
}
