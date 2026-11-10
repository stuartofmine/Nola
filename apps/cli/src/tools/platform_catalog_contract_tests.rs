// SPDX-License-Identifier: Apache-2.0
// Mac permission receipts must authenticate every actual shared Host tool,
// without inventing handlers. This is not a declaration of OS authorization.

use std::collections::BTreeSet;

fn swift_ids(name: &str) -> BTreeSet<String> {
    let source = include_str!("../../../macos/Services/MacCoreToolPolicy.swift");
    let marker = format!("private static let {name}");
    let (_, tail) = source.split_once(&marker).expect("Mac policy partition exists");
    let (_, values) = tail.split_once("= [").expect("partition is a literal array");
    let (values, _) = values.split_once(']').expect("partition terminator exists");
    let values: Vec<_> = values.split('"').enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, value)| value.to_owned()).collect();
    let ids: BTreeSet<_> = values.iter().cloned().collect();
    assert_eq!(ids.len(), values.len(), "duplicate tool in {name}");
    ids
}

#[test]
fn mac_permission_catalog_matches_registered_host_tools() {
    let mut native = swift_ids("macProductivityTools");
    native.extend(swift_ids("webTools"));
    native.extend(swift_ids("unifiedBrowserTools"));
    native.extend(swift_ids("browserActions").into_iter().map(|id| format!("chrome_native_{id}")));
    native.extend(swift_ids("computerActions").into_iter().map(|id| format!("computer_{id}")));
    let shared = swift_ids("localFileAndProcessTools");
    assert!(shared.is_disjoint(&native), "native and shared partitions must not overlap");
    let core_owned = swift_ids("coreOwnedTools");
    assert!(core_owned.is_disjoint(&shared) && core_owned.is_disjoint(&native));
    let mut actual: BTreeSet<_> = super::specs().into_iter().map(|spec| spec.name.to_string()).collect();
    actual.insert(nola_core::agent::generative_ui::capability_descriptor().id);
    let declared: BTreeSet<_> = shared.union(&native).cloned().chain(core_owned).collect();
    assert_eq!(declared, actual,
        "Update Mac receipt partitions when changing the executable Host catalog; no aliases or ghost handlers");
    for id in actual {
        assert!(nola_core::tool_catalog::builtin_tool_metadata(&id).is_some(),
            "Core policy cannot authenticate registered tool {id}");
    }
}
