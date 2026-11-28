# Covalent — design for an iPhone app (`ios/`)

Written 2026-11-15 against Covalent `6ccafac` plus the uncommitted uploads
milestone.

**Status, 2026-11-15:** step 13 is built, apart from its check on a phone.
As built:

- `COVALENT_HOSTS` takes names separated by commas, compared without case;
  each is accepted in `Host` and `Origin`, and nothing else changes.
- The admission event is `admitted`. It carries the user message and its
  pending reply together, and a page skips one it sent itself.
- Voice mode pauses on a real loss of the microphone: its track ending or
  being muted, Safari interrupting audio, or the page returning with the
  microphone gone. It does not pause merely because the tab is hidden, so on a
  Mac it keeps running in a background tab as before.
- `VoiceLoop` gained `pause()`, with a test, so the page and the later Swift
  port share it.

Not checked: anything on an iPhone, Tailscale Serve's `Host` header, and the
wake lock on a real phone.

**Status, 2026-11-15:** step 14's proof is built and compiles for iPhone and
the simulator; it has not run anywhere yet. As built:

- `ios/CovalentVoice` (a Swift package, `swift test` on the Mac): `Server`
  for the routes, `Pcm16` for WAV out and speech in, and `SpeechDetector`,
  with six tests.
- `ios/Covalent`: `CovalentApp.swift` (one screen: address, chat, Start, a
  status line and a level meter) and `VoiceSession.swift` (the audio session,
  the engine with voice processing, and a simple listen, send, speak cycle;
  talking over a reply stops and cancels it; a call or Siri pauses it).
- The Xcode project is hand-written in Xcode's folder-synchronized format, so
  files added under `ios/Covalent/` need no project edit. Bundle identifier
  `local.covalent.app`, iOS 17 and up, Swift 5 language mode.
- The page's retry when a reply restarts and its loop tests came later, with
  step 15a.

The check on the phone, with Covalent reachable over Tailscale:

1. Start talking, ask a question, and hear the answer from the speaker.
2. Lock the phone mid-answer; it keeps speaking. Ask another question while
   locked; it answers.
3. Open another app that does not play sound (Notes); ask a question; it
   answers.
4. Talk over a long answer; it stops, and the chat on the Mac shows the reply
   stopped.
5. Note the iOS version, whether echo made it interrupt itself, and roughly how
   long from the end of a question to the first word.

**Status, 2026-11-21:** the proof ran in the iPhone simulator, not yet on a
phone. Found there:

- iOS stops the audio engine when the audio configuration changes, and in the
  simulator does so right after it starts. The app now starts it again.
- With no echo cancellation between the Mac's speakers and microphone, the app
  heard its own reply as the user talking, stopped it, and sent the clipped
  words as the next message. Whether a phone's speaker does the same is for
  the phone to show; a guard waits on that.

**Status, 2026-11-22:** the app ran on an iPhone 17 Pro. Found there:

- Echo cancellation silences a reply well once it has settled, but settling
  took from half a second to six seconds, and until then the reply reached the
  microphone as loud as the user. The app took it for the user, stopped the
  reply and sent the words as the next message. No bar on loudness separates
  the two, so while a reply plays the microphone is muted and iOS's own
  detector of speech reports the user talking over it. It reported nothing
  through a reply's echo and did report a voice in the room.
- The first reply after voice mode started was silent. iOS restarts the audio
  engine once voice processing is on, and a player left playing across that
  restart plays nothing. The app now stops and starts the player with the
  engine.
- Read aloud in the page was silent with the phone's ringer off, in the app
  and in Safari. The page now marks it as playback.

Step 15 is built in four parts, each checked on the Mac or in the simulator,
and the phone is tested once at the end:

| Part | What | Checked by |
|---|---|---|
| 15a (built) | `VoiceLoop.swift`, the page's turn-taking rules with its ten scenarios; `VoiceSession` is now only the microphone, speaker and server | `swift test` |
| 15b (built) | The first-run screen and the chat in a web view | The simulator |
| 15c (built) | The bridge and the voice bar | Web tests and the simulator |
| 15d (built) | Speech streamed to `/api/listen` while spoken, as the page now does | `swift test`, then a spoken turn |

As built in 15a: whether a reply is still being written is asked of the server
(`GET /api/chats/{id}`), since the app has no event stream of its own, so the
loop's `pending` is asynchronous where the page's is not. A failed send is
reported like any other failure; the page reports its own.

As built in 15b:

- `Setup.swift` asks for the address and keeps it once `GET /api/voice`
  answers as Covalent. A bare name is taken to be HTTPS.
- `ChatView.swift` shows the page. A load that fails offers to try again or
  change the address, which is the only way to change it; there is no
  settings entry.
- Any other site, a link that asks for a new window and a download all open in
  Safari, which can save a file to Files. Downloads inside the app are still
  step 16.
- The page itself is fetched fresh at each start, since Covalent serves it
  with no cache headers and the web view otherwise keeps a stale copy.
- The page's text fields are 16px on touch screens, because iOS zooms into a
  smaller one and stays zoomed. This is the one change outside `ios/`.
- The proof screen is gone.
- Not tried: the on-screen keyboard (the simulator had a hardware one), a
  download, taking a photo to attach, and anything on a phone.

As built in 15c:

- The bridge is `web/src/nativeApp.ts` and the web view in `ChatView.swift`.
  Inside the app the page posts `{voice: <chat id>}` to start voice mode and
  `{voice: null}` when it ends it itself, by switching chats for one. The app
  sends the window event `covalent-voice-ended` each time its voice mode ends,
  with the reason as the event's detail when something failed; the page shows
  the reason as its usual notice.
- The page creates the chat before it asks, so starting voice mode on a new
  chat and ending it without speaking leaves an empty chat behind. The page's
  own voice mode creates the chat with the first message.
- `VoiceBar.swift` lies over the foot of the page, where the page leaves 76
  points for it and hides its footnote. It shows the status, a microphone
  meter and a turn's error, with mute and end; tapping the status interrupts a
  reply or resumes after a pause. Putting the bar under a shorter web view was
  tried first and left the page out of place when a field had focus.
- If iOS ends the page's process while the app is behind another, the page
  reloads without knowing voice mode is on: the conversation carries on and
  the bar stays, but the message box shows behind it until voice mode ends.
- Not tried: a spoken turn through the bridge, and anything on a phone.

As built in 15d:

- `Listen.swift` is a port of `web/src/listen.ts` over a `URLSession`
  WebSocket, tested against a WebSocket listener on a loopback port. The
  detector hands over what it heard of a turn when the turn starts, so
  `Listen` keeps no audio of its own.
- A turn's words come from the stream; when the stream fails, or the words
  take more than 5 s, the whole recording is sent as before.
- Checked against a running Covalent with a sentence synthesized to a file and
  fed through the detector in real time: the streamed words came 0.54 s after
  the pause, the whole recording's after 2.23 s. Not tried with a voice.

This is a design, not a plan with settled steps: the parts marked "to verify"
have not been tried on a device. When code exists and disagrees with this
page, the code is right.

## Goal

Talk to Covalent hands-free from an iPhone, and keep talking when the screen
locks or another app is in front, the way a native voice app does. Everything
else Covalent does (chats, files, images, uploads) should be there too,
without being built twice.

Not in this design: the App Store, iPad layouts, Android, notifications, a
lock-screen Live Activity, CarPlay, offline use, and running Covalent's server
anywhere but the Mac.

## Why a web page cannot do this

iOS suspends a web page's audio and may stop its microphone once the page is
not in front, and a page cannot ask for background audio. A native app can: it
declares the `audio` background mode, and while its audio session is recording
or playing, iOS keeps it running with the screen locked or another app in
front.

## The shape: a web view for the chat, native code for voice

| Approach | Background voice | Cost |
|---|---|---|
| A wrapper that only shows the page | Not dependable: the web view's microphone and audio are still the page's | Smallest |
| **Web view for the chat, voice mode in Swift (chosen)** | Dependable: the app owns the microphone and the speaker | Medium |
| Everything rewritten in SwiftUI | Dependable | Large, and every later feature is built twice |

The chosen shape works because voice mode already goes through three server
routes, which a native client can call as the page does:

| Step | Route | Shape |
|---|---|---|
| Transcribe | `POST /api/transcribe` | Audio in the body (WAV is accepted), `{text}` back |
| Send | `POST /api/chats/{id}/messages` with `spoken: true` | The chat's messages, ending with the pending reply |
| Speak | `GET /api/chats/{id}/messages/{reply}/speech` | Raw 16-bit mono PCM at 24 kHz, streamed as the reply is written |
| Stop | `POST /api/chats/{id}/cancel` | |

So the app is two things in one window:

- **The chat** is Covalent's own page in a `WKWebView`. Files, images,
  uploads, the model picker and everything added later come for free.
- **Voice mode** is Swift. The page's voice button asks the app to start it;
  the app listens, sends and speaks through the routes above, and shows a
  small native bar (status, mute, end) over the page. The thread behind it
  keeps updating because the page hears the same server events.

## What has to exist first: the phone reaching the Mac

The app, like the page, needs an HTTPS address for Covalent that the phone can
reach. Today the server listens on `127.0.0.1` and refuses any request not
addressed to a loopback name.

- **Tailscale Serve** on the Mac publishes `https://<mac>.<tailnet>.ts.net`
  to your own devices only, with a real certificate, and forwards to
  `127.0.0.1:7420`. Covalent keeps listening on loopback.
- **Covalent** gains one setting, `COVALENT_HOSTS`, a list of extra host names
  it answers to. The loopback check accepts those names in `Host` and
  `Origin` and still refuses everything else.
- **Covalent has no login.** With this on, every device on the tailnet can
  read and drive every chat and its files. Tailscale Funnel, which publishes
  to the whole internet, must never be pointed at Covalent. The README will
  say both.

To verify: that Tailscale Serve passes the `ts.net` name through as `Host`,
and that server-sent events and the streamed speech pass through it unbuffered.

## Changes outside `ios/`

Small, and useful without the app:

1. **`COVALENT_HOSTS`**, as above (server).
2. **An event when a message is admitted** (server). Today the page learns of
   a new user message and its pending reply only from its own send. A message
   sent by the app's voice mode, or from another tab, needs an event so every
   open page shows it.
3. **Wake lock and pause-and-resume in the page's voice mode** (web). Keeps
   the screen on while voice mode is open in Safari, and recovers when the
   microphone is taken away. This makes the page usable on the phone before
   the app exists, and stays as the fallback.
4. **The bridge** (web). When the page runs inside the app, its voice button
   posts a message to the app instead of loading the page's own voice mode.
   The page creates the chat first if there is none, since it knows the chosen
   model, and passes the chat's ID.

## The app

### Layout

```
ios/
  Covalent.xcodeproj
  Covalent/                 the app target
    CovalentApp.swift       entry point and the one window
    Setup.swift             first-run screen: the server's address, checked with /api/voice
    ChatView.swift          the WKWebView, the bridge, permissions, downloads
    VoiceBar.swift          the native bar: status, level, mute, end
    VoiceSession.swift      wires microphone, detector, server and speaker to the loop
    Info.plist              background audio, the microphone sentence
  CovalentVoice/            a Swift package with no UIKit, testable on a Mac
    VoiceLoop.swift         the turn-taking rules, ported from web/src/voiceLoop.ts
    SpeechDetector.swift    start and end of speech from microphone levels
    Server.swift            the four routes, as async functions
    Wav.swift               16 kHz mono samples to a WAV body
    Tests/                  the loop's and the detector's scenarios
```

One file per concern, as in the rest of Covalent. The package holds everything
that does not need a phone, so its tests run with `swift test` on the Mac.

### Audio

- **Session.** Category `playAndRecord`, mode `voiceChat`, default to the
  speaker, Bluetooth allowed. It is activated when voice mode starts and
  deactivated when it ends, so the app holds the microphone only then.
- **Echo cancellation.** Voice processing is turned on for the audio engine's
  input. Without it the phone's speaker would be heard by its own microphone:
  the reply would interrupt itself, and talking over a reply would not work.
  It needs up to some seconds to settle on a reply, and until then the reply
  is as loud in the microphone as the user. So while a reply plays the
  microphone is muted and iOS reports the user speaking over it
  (`setMutedSpeechActivityEventListener`), which stops the reply. What the
  user said before the microphone came back, about a second, is not heard.
- **Listening.** A tap on the engine's input, converted to 16 kHz mono, feeds
  the detector and a buffer of the current utterance.
- **Speaking.** The speech route's bytes are scheduled on a player node as
  they arrive, so the first words play while the rest is still being written.
  "Speaking" begins when the first buffer is scheduled.
- **Background.** `UIBackgroundModes: audio`. Voice mode must be started with
  the app in front; after that it continues locked or behind another app.
- **Interruptions.** A phone call or Siri pauses voice mode and shows
  "Paused"; when iOS says the interruption ended, it resumes listening.
  Headphones or AirPods connecting or leaving re-reads the route and carries
  on.

### Detecting speech

The page uses the Silero model in the browser. The app starts simpler: a
detector on the level of the echo-cancelled microphone signal, with the same
shape of rules the page uses (speech must last a moment to count; 800 ms of
quiet ends a turn). It is plain arithmetic, has no dependency, and is tested
with recorded level sequences.

If it proves too eager or too deaf in real rooms, the Silero model can replace
it behind the same two callbacks (speech started, utterance ended). That adds
ONNX Runtime as the app's one dependency. See open point 2.

### Turn-taking

`VoiceLoop.swift` is a port of `web/src/voiceLoop.ts`, with the same rules
and the same test scenarios:

- An utterance starts a turn and replaces an unfinished one.
- Talking over a reply stops the speech, and cancels the reply if it is still
  being written.
- A message overtaken by a newer utterance has its reply cancelled and never
  read.
- A speech stream that ends while the reply is being rewritten is asked for
  again, at most five times.
- Ending voice mode stops the speech and lets the reply finish as text.

The rules now live in two languages. A change to one must be made in the
other; each file says so at the top, and the shared scenario names make a
missing one visible.

### The web view

- Loads the configured address. Only that origin is allowed in the view; any
  other link opens in Safari.
- **Bridge.** One message handler, `covalent`. The page posts
  `{voice: "<chat id>"}` to start voice mode; the app tells the page when
  voice mode ends so its button returns.
- **Permissions.** Dictation in the composer still runs in the page, so the
  view grants the microphone to Covalent's origin without a second prompt.
- **Uploads** work as in Safari: the file picker, the photo library and the
  camera. **Downloads** need a delegate: a downloaded file goes to the share
  sheet, from which it can be saved to Files.
- **Coming back.** iOS suspends the view's page while the app is behind
  another. The page already reloads what it shows when its event stream
  reconnects, so the thread is current on return.

### First run

One screen asks for Covalent's address, checks it with `GET /api/voice`, and
saves it. A wrong or unreachable address says which it is. Afterwards the
address can be changed only from the screen shown when Covalent can't be
reached. That screen also appears when the address answers with an error, as a
proxy does when the Covalent behind it has moved.

## What it needs from you

- **Xcode.** This Mac has only the command-line tools (Swift 5.9, macOS
  14.2.1). An Xcode new enough for your iPhone's iOS version may also need a
  newer macOS. The app cannot be built until that is in place.
- **Signing.** A free Apple ID installs the app for 7 days at a time. The
  Apple Developer Program ($99 a year) makes that a year and adds TestFlight.
- **Tailscale** on the Mac and the phone.
- **Your phone and your ears.** The simulator has no real microphone path,
  echo or lock screen. Each round of audio work needs a run on the iPhone.

## Steps

| Step | What | Done when |
|---|---|---|
| 13 | Phone access and the page: `COVALENT_HOSTS`, the admission event, wake lock, pause and resume | Voice mode in Safari on the iPhone holds a conversation with the screen staying on, and recovers after a manual lock |
| 14 | A proof: a bare app with one button that listens and speaks against one chat | A conversation continues with the phone locked and with another app in front, and talking over a reply stops it |
| 15 | The app: the web view, the bridge, the voice bar, the loop and its tests, first run | Everything in step 14 from inside the real chat, with the thread updating behind the bar |
| 16 | The rough edges: calls, AirPods, downloads, a lock-screen stop control | A phone call mid-conversation pauses and resumes it; a downloaded file reaches Files |

Step 14 exists to fail early. If background listening, echo cancellation or
streaming playback does not behave on your phone, that is known after a few
hundred lines, not after the whole app.

Rough size: step 13 about 150 lines; step 14 about 300 of Swift; step 15
about 700 more with tests; step 16 about 200.

## Testing

- `swift test` in `ios/CovalentVoice`: the loop's nine scenarios against fake
  microphone, server and speaker; the detector against level sequences; the
  WAV writer against a known file. No network, no device.
- Covalent's Rust tests gain the host setting and the admission event.
- Everything about real audio is checked by hand on the iPhone, from a short
  written list (locked, other app in front, a call, AirPods in and out, talk
  over a reply), and the result is written down with the iOS version.

## To verify on the phone

- That listening continues with the screen locked and behind another app for
  many minutes, not just at first.
- That echo cancellation is good enough to talk over a reply on the speaker.
- Time from the end of speech to the first sound. The page measured 5.8 s on
  the Mac; the app changes nothing on the server side of that.
- Battery use over a long conversation.
- That iOS does not stop the app when it has been silent and listening for a
  long time.

## Open points

1. **The minimum iOS version.** Planned: iOS 17. It depends on your phone.
2. **The speech detector**: levels first, as planned, or Silero from the
   start, which matches the page and adds a dependency.
3. **Transcription** through Covalent's route, as planned, or Apple's
   on-device recognition, which is free and may be faster but differs from
   what the page hears.
4. **Whether the page's voice mode stays** once the app exists. Planned: yes,
   for the Mac and as the phone's fallback.
5. **The project file**: an Xcode project checked in, as planned, or one
   generated from a short description by a tool, which is easier to review
   and is one more thing to install.
6. **A stop control on the lock screen** is in step 16; it could be dropped.
