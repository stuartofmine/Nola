# Software distribution and updates

[简体中文](SOFTWARE_UPDATE_ARCHITECTURE.md) · English

Nola distributes complete artifacts through GitHub Releases. The packaged source configuration specifies the feed URL, channel and public verification key. The Rust update Host verifies a signed feed and selects an offer for the current platform, architecture, distribution and Store contract. Publishing metadata and installing an artifact are separate operations. [Source configuration](../../../release/update-source.json) · [Update Host](../../../core/nola-update-host/src/lib.rs)

1. An explicit check loads the compiled source and verifies the feed signature, validity interval and compatibility. A disabled source, current version, missing target, incompatible data and a verification error have distinct outcomes.
2. When the user requests a download, revalidate the offer identity. A changed release requires a fresh choice. Fetch over HTTPS using the update client’s credentials policy and redirect restrictions.
3. Stream to a temporary file, verify the expected length and SHA-256, then publish the completed file without overwriting an existing destination. An interrupted or mismatched download cannot produce an installable result.
4. Hand the verified offer to the platform. The installed application checks the Store contract again at startup.

| Surface | Installation handoff |
| --- | --- |
| macOS direct distribution | Present the verified DMG or PKG for the system installer. |
| iOS | Open the approved App Store or TestFlight destination. |
| Android direct / store | Check the APK identity and build before FileProvider handoff. Store builds open the Play Store offer and omit direct-install permission. |
| Windows | Save the selected desktop installer and reveal it to the user. The Tauri shell uses its local Rust Host. |
| Browser | An update concerns the connected Host. Web assets ship with that Host. |
| CLI | Check the feed or download the archive to the requested destination. The user installs it after stopping affected processes. |

Feed signatures authenticate release metadata. Platform signing, notarization or store approval is a separate release check for each artifact. An offer in a feed does not establish that an installer has passed a native install-and-relaunch test. [Release engineering](../../../release/README.md) · [Data compatibility](VERSION_COMPATIBILITY.en.md)
