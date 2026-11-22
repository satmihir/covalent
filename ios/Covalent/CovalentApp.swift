import CovalentVoice
import SwiftUI

@main
struct CovalentApp: App {
    /// address is where Covalent answered when it was last set up; empty before the first run.
    @AppStorage("address") private var address = ""
    @State private var changing = false

    var body: some Scene {
        WindowGroup {
            if !changing, let server = Server(address: address) {
                ChatView(server: server) { changing = true }
            } else {
                Setup(address: address) {
                    address = $0
                    changing = false
                }
            }
        }
    }
}
