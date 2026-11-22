import CovalentVoice
import SwiftUI

/// Setup is the first-run screen: it asks where Covalent is, and keeps the address once
/// Covalent has answered there.
struct Setup: View {
    @State var address: String
    let done: (String) -> Void

    @State private var checking = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("https://mac.tail1234.ts.net", text: $address)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(connect)
                } header: {
                    Text("Covalent's address")
                } footer: {
                    Text("The name your devices reach Covalent by, such as its Tailscale name. Covalent must be started with that name in COVALENT_HOSTS.")
                }

                Section {
                    Button(checking ? "Connecting…" : "Connect", action: connect)
                        .disabled(checking || address.isEmpty)
                    if let error {
                        Text(error).foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Covalent")
        }
    }

    private func connect() {
        guard let server = Server(address: address) else {
            error = "That isn't an address. It looks like https://mac.tail1234.ts.net."
            return
        }
        checking = true
        error = nil
        Task {
            do {
                _ = try await server.voice()
                done(server.base.absoluteString)
            } catch {
                self.error = error.localizedDescription
            }
            checking = false
        }
    }
}
