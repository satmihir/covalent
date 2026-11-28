import CovalentVoice
import SwiftUI
import WebKit

/// ChatView is the chat: Covalent's own page, so everything the page does is here too. Voice
/// mode alone is the app's: the page asks for it, and its bar sits under the page.
struct ChatView: View {
    let server: Server
    let changeAddress: () -> Void

    @State private var failure: String?
    @State private var voice = VoiceSession()

    var body: some View {
        if let failure {
            ContentUnavailableView {
                Label("Can't reach Covalent", systemImage: "wifi.slash")
            } description: {
                Text(failure)
            } actions: {
                // Clearing the failure makes a new web view, which loads the page again.
                Button("Try again") { self.failure = nil }
                Button("Change address", action: changeAddress)
            }
        } else {
            WebPage(server: server, voice: voice) {
                voice.stop()
                failure = $0
            }
            // The bar lies over the foot of the page, where the page leaves it room. Resizing
            // the web view instead leaves its content out of place when a field has focus.
            .overlay(alignment: .bottom) {
                if voice.status != .off {
                    VoiceBar(voice: voice).background(Color(WebPage.background))
                }
            }
            // The web view moves its own content above the keyboard.
            .ignoresSafeArea(.keyboard)
            .background(Color(WebPage.background).ignoresSafeArea())
        }
    }
}

/// WebPage shows Covalent's page and keeps the web view to it: any other site, a link that asks
/// for a new window, and a download all open in Safari. It is also the bridge to the page,
/// whose other half is web/src/nativeApp.ts.
private struct WebPage: UIViewRepresentable {
    /// background is the page's own background, shown around it and before it loads.
    static let background = UIColor { traits in
        traits.userInterfaceStyle == .dark
            ? UIColor(red: 0x18 / 255, green: 0x18 / 255, blue: 0x1b / 255, alpha: 1) : .white
    }

    let server: Server
    let voice: VoiceSession
    let failed: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(server: server, voice: voice, failed: failed)
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.allowsInlineMediaPlayback = true
        // The page sees this handler and leaves voice mode to the app.
        configuration.userContentController.add(context.coordinator, name: "covalent")
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = context.coordinator
        view.uiDelegate = context.coordinator
        // The page keeps each chat in the history, so a swipe goes back to the last one.
        view.allowsBackForwardNavigationGestures = true
        view.isOpaque = false
        view.backgroundColor = Self.background
        // Covalent's page names its scripts and styles by their contents, so only the page
        // itself can go stale; it is fetched fresh each time the app starts.
        // The page is told each time voice mode ends, and why if it failed, so that its
        // message box comes back.
        voice.ended = { [weak view] reason in
            view?.callAsyncJavaScript(
                "window.dispatchEvent(new CustomEvent('covalent-voice-ended', { detail: reason }))",
                arguments: ["reason": reason ?? NSNull()], in: nil, in: .page,
                completionHandler: nil)
        }
        view.load(URLRequest(url: server.base, cachePolicy: .reloadIgnoringLocalCacheData))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler {
        private let server: Server
        private let voice: VoiceSession
        private let failed: (String) -> Void
        private var base: URL { server.base }

        init(server: Server, voice: VoiceSession, failed: @escaping (String) -> Void) {
            self.server = server
            self.voice = voice
            self.failed = failed
        }

        /// The page posts {voice: chat ID} to start voice mode in that chat, and {voice: null}
        /// to end it.
        func userContentController(
            _ controller: WKUserContentController, didReceive message: WKScriptMessage
        ) {
            guard message.frameInfo.isMainFrame, let body = message.body as? [String: Any] else {
                return
            }
            guard let chatId = body["voice"] as? String else { return voice.stop() }
            Task { await voice.start(server: server, chatId: chatId) }
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async
            -> WKNavigationActionPolicy
        {
            guard let url = action.request.url else { return .cancel }
            // A link that asks for a new window has no frame to load in.
            guard let frame = action.targetFrame else { return await openInSafari(url) }
            if !frame.isMainFrame { return .allow }
            let ours = url.scheme == base.scheme && url.host() == base.host() && url.port == base.port
            if ours, !action.shouldPerformDownload { return .allow }
            return await openInSafari(url)
        }

        private func openInSafari(_ url: URL) async -> WKNavigationActionPolicy {
            await UIApplication.shared.open(url)
            return .cancel
        }

        /// The page's dictation asks for the microphone. iOS has asked for the app, so Covalent's
        /// page is not asked a second time.
        func webView(
            _ webView: WKWebView, decideMediaCapturePermissionsFor origin: WKSecurityOrigin,
            initiatedBy frame: WKFrameInfo, type: WKMediaCaptureType
        ) async -> WKPermissionDecision {
            let ours = origin.protocol == base.scheme && origin.host == base.host()
                && origin.port == base.port ?? 0
            return ours && type == .microphone ? .grant : .deny
        }

        /// An address that answers with an error, such as a proxy with no Covalent behind it,
        /// has failed too: shown as a page it is blank, with no way to change the address.
        func webView(_ webView: WKWebView, decidePolicyFor response: WKNavigationResponse) async
            -> WKNavigationResponsePolicy
        {
            guard response.isForMainFrame,
                let status = (response.response as? HTTPURLResponse)?.statusCode, status >= 400
            else { return .allow }
            failed("The address answered with an error (\(status)), not with Covalent.")
            return .cancel
        }

        func webView(
            _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!,
            withError error: Error
        ) {
            // A link handed to Safari arrives here as a cancelled load, which is not a failure.
            let code = (error as NSError).code
            if code == NSURLErrorCancelled || code == 102 { return }
            failed(error.localizedDescription)
        }

        /// iOS may end the page's process while the app is behind another; the page comes back.
        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            webView.reload()
        }
    }
}
