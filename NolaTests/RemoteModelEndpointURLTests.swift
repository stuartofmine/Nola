import Testing
@testable import Nola

struct RemoteModelEndpointURLTests {
    @Test
    func acceptsHTTPAndHTTPSWithHosts() {
        #expect(RemoteModelEndpointURL.isValid("https://api.example.com/v1"))
        #expect(RemoteModelEndpointURL.isValid(" http://127.0.0.1:11434/v1 "))
    }

    @Test
    func rejectsUnsupportedOrAmbiguousAddresses() {
        for value in [
            "", "api.example.com/v1", "file:///tmp/model", "ftp://api.example.com",
            "https://user@api.example.com/v1", "https://user:secret@api.example.com/v1",
            "https://api.example.com/v1#fragment", "https:///v1",
        ] {
            #expect(!RemoteModelEndpointURL.isValid(value), "Unexpectedly accepted: \(value)")
        }
    }
}
