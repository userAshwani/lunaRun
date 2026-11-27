//! The tray's menu (FR-AP-02, D-043): what it offers for which state, and that a click maps back to what was meant. The
//! menu itself is drawn by the operating system and is checked in the real window (TRACKER section 7).

use lunarun_lib::tray::{EN, TrayAction, TrayItem, TrayProject, menu_model, status_line};

fn project(id: &str, has_site: bool, served: bool, running: bool) -> TrayProject {
    TrayProject {
        id: id.to_owned(),
        name: format!("Projet {id} é"),
        has_site,
        served,
        running,
        address: has_site.then(|| "http://127.0.0.1:8101".to_owned()),
    }
}

/// The actions of a menu, with whether each is enabled, flattened (submenus included).
fn actions(items: &[TrayItem]) -> Vec<(TrayAction, bool)> {
    let mut out = Vec::new();
    for item in items {
        match item {
            TrayItem::Action {
                action, enabled, ..
            } => out.push((action.clone(), *enabled)),
            TrayItem::Submenu { items, .. } => out.extend(actions(items)),
            TrayItem::Text(_) | TrayItem::Separator => {}
        }
    }
    out
}

fn enabled(items: &[TrayItem], action: &TrayAction) -> bool {
    actions(items)
        .into_iter()
        .find(|(a, _)| a == action)
        .unwrap_or_else(|| panic!("{action:?} is not in the menu"))
        .1
}

#[test]
fn the_menu_starts_with_the_status_and_open_and_ends_with_stop_all_and_quit() {
    let items = menu_model(&[], 0, &EN);
    assert_eq!(items[0], TrayItem::Text("Nothing is running".to_owned()));
    assert_eq!(
        items[1],
        TrayItem::Action {
            label: "Open lunaRun".to_owned(),
            action: TrayAction::Open,
            enabled: true
        }
    );
    assert!(items.contains(&TrayItem::Text("No projects yet".to_owned())));
    let last = &items[items.len() - 1];
    assert_eq!(
        *last,
        TrayItem::Action {
            label: "Quit lunaRun".to_owned(),
            action: TrayAction::Quit,
            enabled: true
        }
    );
    // Nothing runs: there is nothing to stop.
    assert!(!enabled(&items, &TrayAction::StopAll));
}

#[test]
fn the_status_line_counts_the_running_services() {
    assert_eq!(status_line(0, &EN), "Nothing is running");
    assert_eq!(status_line(1, &EN), "1 service is running");
    assert_eq!(status_line(7, &EN), "7 services are running");
    assert!(enabled(&menu_model(&[], 3, &EN), &TrayAction::StopAll));
}

#[test]
fn each_project_offers_only_what_can_be_done_in_its_state() {
    let projects = [
        project("idle", true, false, false),
        project("live", true, true, true),
        project("down", true, true, false),
        project("nophp", false, false, false),
    ];
    let items = menu_model(&projects, 4, &EN);

    // Not started: Start, the terminal; no browser (nothing answers).
    assert!(enabled(&items, &TrayAction::Start("idle".into())));
    assert!(enabled(&items, &TrayAction::Terminal("idle".into())));
    assert!(!enabled(&items, &TrayAction::Browser("idle".into())));
    // Started and answering: Stop and the browser.
    assert!(enabled(&items, &TrayAction::Stop("live".into())));
    assert!(enabled(&items, &TrayAction::Browser("live".into())));
    // Started but not answering: Stop, no browser.
    assert!(enabled(&items, &TrayAction::Stop("down".into())));
    assert!(!enabled(&items, &TrayAction::Browser("down".into())));
    // No PHP: nothing can be started or opened, and the menu still lists it.
    assert!(!enabled(&items, &TrayAction::Start("nophp".into())));
    assert!(!enabled(&items, &TrayAction::Terminal("nophp".into())));

    // A started project has Stop and no Start; one that is not has Start and no Stop.
    let all = actions(&items);
    assert!(
        !all.iter()
            .any(|(a, _)| *a == TrayAction::Start("live".into()))
    );
    assert!(
        !all.iter()
            .any(|(a, _)| *a == TrayAction::Stop("idle".into()))
    );

    // The labels say which are running.
    let labels: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            TrayItem::Submenu { label, .. } => Some(label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        labels,
        [
            "Projet idle é",
            "Projet live é (running)",
            "Projet down é (not answering)",
            "Projet nophp é"
        ]
    );
}

#[test]
fn a_click_maps_back_to_exactly_the_action_and_nothing_else_is_understood() {
    for action in [
        TrayAction::Open,
        TrayAction::StopAll,
        TrayAction::Quit,
        TrayAction::Start("4b1e0c".into()),
        TrayAction::Stop("4b1e0c".into()),
        TrayAction::Browser("4b1e0c".into()),
        TrayAction::Terminal("4b1e0c".into()),
    ] {
        assert_eq!(
            TrayAction::parse(&action.id()),
            Some(action.clone()),
            "{action:?}"
        );
    }
    for unknown in ["", "start:", "delete:4b1e0c", "OPEN", "quit:now", "browser"] {
        assert_eq!(TrayAction::parse(unknown), None, "{unknown:?}");
    }
}
