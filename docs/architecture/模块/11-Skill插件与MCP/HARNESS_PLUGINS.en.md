# Pi and DeepSeek Harness plugins

[简体中文](HARNESS_PLUGINS.md) · English

Choose the target Host and runtime when installing a plugin. A repository may contain an ordinary Skill as well as a native plugin. A Skill uses the Skill installer. Pi extensions and DeepSeek Harness plugins require their own installed runtimes.

| Operation | How it works |
| --- | --- |
| Install, list or remove | The CLI invokes the selected runtime in its own state directory. Pi uses PI_CODING_AGENT_DIR. DSH uses its own home, agents directory and profile. Refresh the installed list after a mutation. |
| Open a runtime session | The runtime loads its native commands, services or UI in the selected workspace. Installed native code runs with the operating system privileges of that process. |
| Open a desktop page | Read the explicitly declared page from the installed package on demand. Open it in an isolated view inside the desktop workspace. Uninstalling the package removes its page entries. |
| Manage from a phone | Target an authenticated desktop Host with the appropriate runtime. Local mobile Skills and HTTP MCP continue to use the device’s own capabilities. |

A package can declare a page through `nola.schema_version` and `nola.pages` in its package metadata. The declaration identifies a page, title and local entry. This lets a desktop workspace show a page directly in its sidebar. Pi-drawn components and DSH-specific slots still require the compatible runtime. Installing a package does not automatically publish all of its code as callable tools in a Nola conversation.

For verification, compare the package inventory before and after installation, open the declared page or the actual runtime session, inspect a real interaction, remove the package and reload the inventory. Check missing-runtime and disconnected-Host paths independently. [CLI integration](../../../../crates/nola-modules/src/local_models_plugins.rs) · [Harness regression](../../../../crates/nola-modules/src/local_models_plugins.rs)
