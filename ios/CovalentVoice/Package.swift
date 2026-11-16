// swift-tools-version: 6.0
import PackageDescription

// CovalentVoice holds the parts of voice mode that need no phone, so `swift test` runs them on
// a Mac. The app target adds the microphone, the speaker and the screen.
let package = Package(
    name: "CovalentVoice",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "CovalentVoice", targets: ["CovalentVoice"])],
    targets: [
        .target(name: "CovalentVoice"),
        .testTarget(name: "CovalentVoiceTests", dependencies: ["CovalentVoice"]),
    ],
    swiftLanguageModes: [.v5]
)
