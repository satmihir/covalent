import CovalentVoice
import Testing

@Test(arguments: [
    ("https://mac.tail1234.ts.net", "https://mac.tail1234.ts.net"),
    ("  http://127.0.0.1:7420\n", "http://127.0.0.1:7420"),
    ("mac.tail1234.ts.net", "https://mac.tail1234.ts.net"),
    ("192.168.1.20:7420", "https://192.168.1.20:7420"),
])
func anAddressAsTypedBecomesTheServersBase(typed: String, base: String) {
    #expect(Server(address: typed)?.base.absoluteString == base)
}

@Test(arguments: ["", "   ", "ftp://mac.example", "https://", "not an address"])
func whatIsNotAnHttpAddressIsRefused(typed: String) {
    #expect(Server(address: typed) == nil)
}
