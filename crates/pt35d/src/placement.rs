//! Which workspace each window lives on.
//!
//! One window owns the screen, so every tiled window gets a workspace of its
//! own. A profile with `workspace = N` owns N, and the oldest window of that
//! app lives there. Every other window is a guest: a second terminal, a
//! profile with no workspace, a .desktop app, a window an app opened by
//! itself. A guest never sits on an owned workspace, even one whose app is
//! closed, so no owner ever has to push it out. With the shipped profiles
//! owning 1 to 9, guests live on 10 and up.
//!
//! `plan` is the whole policy. pt35d runs it on every tree change, so any
//! layout ends up there: a launch that lost its race, a pt35d restart, an
//! apps.toml reload. A window that is where the policy wants it never moves.

use pt35_common::apps::AppTable;
use pt35_common::sway::Window;

/// What it takes to make sway match the policy.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// `(window, workspace)`, oldest window first.
    pub moves: Vec<(i64, u8)>,
    /// The window to focus once the moves are made. Without it sway stays on
    /// the workspace a focused window left, and the empty desktop opens.
    pub focus: Option<i64>,
}

impl Plan {
    /// The whole plan as one sway command list, so sway draws only the result.
    pub fn command(&self) -> Option<String> {
        let mut parts: Vec<String> = self
            .moves
            .iter()
            .map(|(id, workspace)| {
                format!("[con_id={id}] move container to workspace number {workspace}")
            })
            .collect();
        if let Some(id) = self.focus {
            parts.push(format!("[con_id={id}] focus"));
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

/// The workspace `app` owns. The first matching profile by id decides:
/// pt35d sends sway's assigns in that order and sway applies the first that
/// matches, so this is where the assign put the window. Not after a
/// `pt35ctl reload` that moved an app: sway keeps the old assign until its
/// own reload, and the window is moved from there.
pub fn home(apps: &AppTable, app: &str) -> Option<u8> {
    apps.apps
        .values()
        .filter(|profile| profile.matches_app(app))
        .find_map(|profile| profile.owned_workspace())
}

/// sway's `assign` rules, in the order `home` reads them. Only for a profile
/// that owns a workspace: an assign by title would put a window where `plan`
/// cannot tell it belongs, and `plan` would move it away again.
pub fn assign_rules(apps: &AppTable) -> Vec<String> {
    apps.apps
        .values()
        .filter_map(|app| Some((app, app.owned_workspace()?)))
        .flat_map(|(app, workspace)| {
            app.sway_criteria_list()
                .into_iter()
                .map(move |criteria| format!("assign {criteria} workspace number {workspace}"))
        })
        .collect()
}

fn owned(apps: &AppTable, workspace: u8) -> bool {
    apps.apps
        .values()
        .any(|profile| profile.owned_workspace() == Some(workspace))
}

/// A window that needs a workspace to itself. A floating window is a dialog
/// over its app. So is a portal's file chooser: a sway rule floats it, but it
/// may map first, and moved away from its app it waits where you cannot see.
fn tiles(window: &Window) -> bool {
    !window.floating && window.workspace > 0 && !window.app.starts_with("xdg-desktop-portal")
}

/// The moves that give every tiled window its place.
///
/// `windows` must include windows asked to close that have not gone yet, and
/// `closing` names them. `front` is the focused workspace.
pub fn plan(windows: &[Window], apps: &AppTable, front: u8, closing: &[i64]) -> Plan {
    let mut tiled: Vec<&Window> = windows.iter().filter(|w| tiles(w)).collect();
    // sway numbers containers as they are made, so a lower id is older.
    tiled.sort_by_key(|w| w.id);

    // Owners take their workspace, and guests keep theirs if no app owns it
    // and nothing older is there. The two never meet: owners only go to owned
    // workspaces, guests only stay on the others.
    let mut held: Vec<u8> = Vec::new();
    let mut places: Vec<(&Window, Option<u8>)> = Vec::new();
    let (going, live): (Vec<&Window>, Vec<&Window>) =
        tiled.iter().partition(|w| closing.contains(&w.id));
    for window in live {
        let place = match home(apps, &window.app) {
            Some(home) if !held.contains(&home) => Some(home),
            _ if !owned(apps, window.workspace) && !held.contains(&window.workspace) => {
                Some(window.workspace)
            }
            _ => None,
        };
        if let Some(place) = place {
            held.push(place);
        }
        places.push((window, place));
    }
    // A window asked to close stays put until it goes: moved, you could be
    // taken along to a workspace about to empty. It holds its workspace so no
    // guest lands there, but a live window of its app still takes their home.
    for window in going {
        held.push(window.workspace);
        places.push((window, Some(window.workspace)));
    }
    places.sort_by_key(|(w, _)| w.id);

    // A dialog goes with its own app: GIMP opened over the Editor takes its
    // splash and its modals along. With no window of its app there, it goes
    // with the workspace: it stays if a tiled window stays, else it goes with
    // the oldest that leaves. A workspace whose dialogs stay is not free.
    // Placing guests below never changes either answer: no guest is given
    // the workspace it is on.
    let loose: Vec<&Window> = windows
        .iter()
        .filter(|w| !tiles(w) && w.workspace > 0)
        .collect();
    let follow: Vec<(i64, Option<i64>)> = loose
        .iter()
        .map(|dialog| {
            let here: Vec<&(&Window, Option<u8>)> = places
                .iter()
                .filter(|(w, _)| w.workspace == dialog.workspace)
                .collect();
            let own: Vec<&(&Window, Option<u8>)> = here
                .iter()
                .copied()
                .filter(|(w, _)| !dialog.app.is_empty() && w.app == dialog.app)
                .collect();
            let with = if own.is_empty() { here } else { own };
            let stays = with.iter().any(|(w, place)| *place == Some(w.workspace));
            let leaver = with
                .iter()
                .find(|(w, place)| *place != Some(w.workspace))
                .map(|(w, _)| w.id);
            (dialog.id, leaver.filter(|_| !stays))
        })
        .collect();
    let blocked: Vec<u8> = loose
        .iter()
        .zip(&follow)
        .filter(|(_, (_, leaver))| leaver.is_none())
        .map(|(dialog, _)| dialog.workspace)
        .collect();

    // Guests with no place go to a free workspace no app owns: the one in
    // front if it is free, for the newest of them, which is the app just
    // launched there. Then the lowest. There is no cap at 9.
    let free = |n: u8, held: &[u8]| {
        n > 0 && !owned(apps, n) && !held.contains(&n) && !blocked.contains(&n)
    };
    if let Some(newest) = places.iter().rposition(|(_, place)| place.is_none()) {
        if free(front, &held) {
            places[newest].1 = Some(front);
            held.push(front);
        }
    }
    for (_, place) in places.iter_mut().filter(|(_, place)| place.is_none()) {
        // None free means 255 windows. It stays where it is, sharing.
        *place = (1..=u8::MAX).find(|n| free(*n, &held));
        held.extend(*place);
    }

    let mut moves: Vec<(i64, u8)> = places
        .iter()
        .filter_map(|(w, place)| place.filter(|p| *p != w.workspace).map(|p| (w.id, p)))
        .collect();
    for (dialog, leaver) in follow {
        let to = leaver.and_then(|id| moves.iter().find(|(m, _)| *m == id).map(|(_, to)| *to));
        if let Some(to) = to {
            moves.push((dialog, to));
        }
    }
    moves.sort_by_key(|(id, _)| *id);

    // The window you were in moved: go with it. With the menu up nothing is
    // focused, and that is the oldest window in front. Otherwise a window put
    // on the workspace in front is the one you are waiting for.
    let moved = |id: i64| moves.iter().any(|(m, _)| *m == id);
    let current = windows
        .iter()
        .find(|w| w.focused)
        .map(|w| w.id)
        .or_else(|| tiled.iter().find(|w| w.workspace == front).map(|w| w.id));
    let focus = current.filter(|id| moved(*id)).or_else(|| {
        places
            .iter()
            .rev()
            .find(|(w, place)| *place == Some(front) && w.workspace != front)
            .map(|(w, _)| w.id)
    });
    Plan { moves, focus }
}

/// Where a launched app's window will end up, so the launch can go there
/// first and the window opens in front. `owns` is the workspace the app
/// owns. A second copy of an owner is a guest, unless the first is closing.
pub fn launch_target(
    windows: &[Window],
    apps: &AppTable,
    owns: Option<u8>,
    front: u8,
    closing: &[i64],
) -> u8 {
    if let Some(slot) = owns {
        let taken = windows
            .iter()
            .any(|w| tiles(w) && !closing.contains(&w.id) && home(apps, &w.app) == Some(slot));
        if !taken {
            return slot;
        }
    }
    let free = |n: u8| n > 0 && !owned(apps, n) && !windows.iter().any(|w| w.workspace == n);
    if free(front) {
        return front;
    }
    (1..=u8::MAX).find(|n| free(*n)).unwrap_or(front)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pt35_common::apps::{AppProfile, Match};

    /// Profiles whose window's app id is their name.
    fn apps<S: AsRef<str>>(slots: &[(S, u8)]) -> AppTable {
        let mut table = AppTable::default();
        for (name, workspace) in slots {
            let name = name.as_ref();
            table.apps.insert(
                name.to_string(),
                AppProfile {
                    exec: name.to_string(),
                    match_: Match {
                        app_id: Some(name.to_string()),
                        ..Match::default()
                    },
                    workspace: *workspace,
                    multiple: name == "foot",
                    ..AppProfile::default()
                },
            );
        }
        table
    }

    /// The shipped layout: 1 to 9 owned, and two apps without a workspace.
    fn shipped() -> AppTable {
        apps(&[
            ("foot", 1),
            ("files", 2),
            ("mousepad", 3),
            ("monitor", 4),
            ("vlc", 5),
            ("imv", 6),
            ("mpv", 7),
            ("chromium", 8),
            ("galculator", 9),
            ("ncdu", 0),
            ("darkwire", 0),
        ])
    }

    fn window(id: i64, app: &str, workspace: u8) -> Window {
        Window {
            id,
            workspace,
            app: app.into(),
            title: String::new(),
            focused: false,
            floating: false,
        }
    }

    fn focused(id: i64, app: &str, workspace: u8) -> Window {
        Window {
            focused: true,
            ..window(id, app, workspace)
        }
    }

    fn dialog(id: i64, app: &str, workspace: u8) -> Window {
        Window {
            floating: true,
            ..window(id, app, workspace)
        }
    }

    /// Apply a plan the way sway would, and the focused workspace after it.
    fn apply(windows: &[Window], plan: &Plan, front: u8) -> (Vec<Window>, u8) {
        let mut after = windows.to_vec();
        for (id, to) in &plan.moves {
            after.iter_mut().find(|w| w.id == *id).unwrap().workspace = *to;
        }
        let mut front = front;
        if let Some(id) = plan.focus {
            for w in &mut after {
                w.focused = w.id == id;
                if w.focused {
                    front = w.workspace;
                }
            }
        }
        (after, front)
    }

    /// Plan, apply, and check that a second look finds nothing to do. A plan
    /// that did not settle would move windows on every sway event.
    fn settle(windows: &[Window], apps: &AppTable, front: u8) -> (Plan, Vec<Window>, u8) {
        let first = plan(windows, apps, front, &[]);
        let (after, front) = apply(windows, &first, front);
        assert_eq!(plan(&after, apps, front, &[]), Plan::default(), "{after:?}");
        (first, after, front)
    }

    #[test]
    fn a_settled_layout_needs_nothing() {
        let windows = [
            window(10, "foot", 1),
            focused(11, "mousepad", 3),
            window(12, "ncdu", 10),
            window(13, "darkwire", 11),
        ];
        assert_eq!(plan(&windows, &shipped(), 3, &[]), Plan::default());
    }

    #[test]
    fn a_guest_opened_over_an_app_moves_and_the_app_stays() {
        // Disk usage, run while the Editor was in front: it maps on 3 and
        // takes focus. The Editor keeps 3, and you go with Disk usage.
        let windows = [window(10, "mousepad", 3), focused(11, "ncdu", 3)];
        let (plan, after, front) = settle(&windows, &shipped(), 3);
        assert_eq!(plan.moves, [(11, 10)]);
        assert_eq!(plan.focus, Some(11));
        assert_eq!(front, 10);
        assert_eq!(after[0].workspace, 3);
    }

    #[test]
    fn an_owner_opening_never_pushes_a_guest_around() {
        // A guest never sits on 5, so Music opening there moves nothing.
        let windows = [window(10, "darkwire", 10), focused(11, "vlc", 5)];
        assert_eq!(plan(&windows, &shipped(), 5, &[]), Plan::default());
    }

    #[test]
    fn a_guest_on_an_owned_workspace_leaves_it_even_while_the_owner_is_closed() {
        // Left there by an older pt35d. You are looking at it, so you go too.
        let windows = [focused(10, "darkwire", 5)];
        let (plan, _, front) = settle(&windows, &shipped(), 5);
        assert_eq!(plan.moves, [(10, 10)]);
        assert_eq!(plan.focus, Some(10));
        assert_eq!(front, 10);
    }

    #[test]
    fn an_owner_opening_on_a_guest_takes_its_workspace() {
        // The guest is already out of place: the owner stays, the guest goes,
        // and focus stays on the app that just opened.
        let windows = [window(10, "darkwire", 5), focused(11, "vlc", 5)];
        let (plan, _, _) = settle(&windows, &shipped(), 5);
        assert_eq!(plan.moves, [(10, 10)]);
        assert_eq!(plan.focus, None);
    }

    #[test]
    fn a_tenth_window_gets_a_tenth_workspace() {
        let mut windows: Vec<Window> = [
            "foot",
            "files",
            "mousepad",
            "monitor",
            "vlc",
            "imv",
            "mpv",
            "chromium",
            "galculator",
        ]
        .iter()
        .zip(1u8..)
        .map(|(app, n)| window(i64::from(n), app, n))
        .collect();
        windows.push(focused(99, "gimp", 1));
        let (plan, _, front) = settle(&windows, &shipped(), 1);
        assert_eq!(plan.moves, [(99, 10)]);
        assert_eq!(front, 10);
    }

    #[test]
    fn a_second_terminal_is_a_guest_and_opens_in_front() {
        // launch went to 10 first. The assign put the new terminal on 1.
        let windows = [window(10, "foot", 1), window(11, "foot", 1)];
        let target = launch_target(&[window(10, "foot", 1)], &shipped(), Some(1), 3, &[]);
        assert_eq!(target, 10);
        let (plan, _, front) = settle(&windows, &shipped(), target);
        assert_eq!(plan.moves, [(11, 10)]);
        assert_eq!(plan.focus, Some(11));
        assert_eq!(front, 10);
    }

    #[test]
    fn the_oldest_terminal_left_goes_home() {
        // The terminal on 1 closed. The one on 10 is the terminal now.
        let windows = [focused(11, "foot", 10), window(12, "foot", 11)];
        let (plan, _, front) = settle(&windows, &shipped(), 10);
        assert_eq!(plan.moves, [(11, 1)]);
        assert_eq!(plan.focus, Some(11));
        assert_eq!(front, 1);
    }

    #[test]
    fn focus_follows_the_window_behind_the_menu() {
        // Terminal #1 closed from the switcher, over terminal #2 on 12. The
        // menu holds the keyboard, so nothing is focused. Left on an empty 12,
        // closing the menu would land on the launcher.
        let windows = [window(11, "foot", 12)];
        let (plan, _, front) = settle(&windows, &shipped(), 12);
        assert_eq!(plan.moves, [(11, 1)]);
        assert_eq!(plan.focus, Some(11));
        assert_eq!(front, 1);
    }

    #[test]
    fn a_window_that_stays_in_front_keeps_you_there() {
        // A guest maps over the Editor while the menu is up.
        let windows = [window(10, "mousepad", 3), window(11, "gimp", 3)];
        let (plan, _, _) = settle(&windows, &shipped(), 3);
        assert_eq!(plan.moves, [(11, 10)]);
        assert_eq!(plan.focus, None);
    }

    #[test]
    fn a_closing_window_is_never_moved_or_followed() {
        // Close all: the terminal on 1 has gone, the one on 10 is going. It
        // would be the terminal now, but moving it would take you along.
        let windows = [focused(11, "foot", 10)];
        assert_eq!(plan(&windows, &shipped(), 10, &[11]), Plan::default());
    }

    #[test]
    fn an_app_opened_again_while_its_window_closes_goes_home() {
        // The Editor was closed and picked again within the close grace. The
        // new one takes 3, the old one stays until it goes. No jumps.
        let windows = [window(10, "mousepad", 3), focused(20, "mousepad", 3)];
        assert_eq!(plan(&windows, &shipped(), 3, &[10]), Plan::default());
        assert_eq!(
            launch_target(&windows[..1], &shipped(), Some(3), 12, &[10]),
            3
        );
        assert_eq!(
            launch_target(&windows[..1], &shipped(), Some(3), 12, &[]),
            12,
            "a second copy, while the first stays: the empty workspace in front"
        );
    }

    #[test]
    fn a_closing_window_still_holds_its_workspace() {
        let windows = [
            window(10, "ncdu", 10),
            window(11, "mousepad", 3),
            focused(12, "gimp", 3),
        ];
        let (plan, _, _) = settle(&windows, &shipped(), 3);
        assert_eq!(plan.moves, [(12, 11)]);
        assert_eq!(
            super::plan(&windows, &shipped(), 3, &[10]).moves,
            [(12, 11)],
            "not onto the one that is closing"
        );
    }

    #[test]
    fn a_mixed_up_layout_sorts_itself_out() {
        // Swapped and squatting, as the old placement could leave it.
        let windows = [
            window(10, "files", 3),
            focused(11, "mousepad", 2),
            window(12, "ncdu", 1),
            window(13, "darkwire", 12),
        ];
        let (plan, after, front) = settle(&windows, &shipped(), 2);
        assert_eq!(plan.moves, [(10, 2), (11, 3), (12, 10)]);
        assert_eq!(plan.focus, Some(11));
        assert_eq!(front, 3);
        assert_eq!(after[3].workspace, 12, "a guest in place stays");
    }

    #[test]
    fn a_guest_keeps_its_workspace_past_a_gap() {
        let windows = [window(10, "ncdu", 12)];
        assert_eq!(plan(&windows, &shipped(), 12, &[]), Plan::default());
    }

    #[test]
    fn the_older_guest_keeps_a_shared_workspace() {
        let windows = [window(10, "ncdu", 10), focused(11, "darkwire", 10)];
        let (plan, _, _) = settle(&windows, &shipped(), 10);
        assert_eq!(plan.moves, [(11, 11)]);
        assert_eq!(plan.focus, Some(11));
    }

    #[test]
    fn a_guest_goes_to_the_empty_workspace_in_front() {
        // You are on the launcher on 12. Something maps over the Editor.
        let windows = [window(10, "mousepad", 3), window(11, "gimp", 3)];
        let (plan, _, front) = settle(&windows, &shipped(), 12);
        assert_eq!(plan.moves, [(11, 12)]);
        assert_eq!(plan.focus, Some(11));
        assert_eq!(front, 12);
    }

    #[test]
    fn an_owned_workspace_in_front_is_not_given_to_a_guest() {
        // On Music's empty workspace with the launcher up.
        let windows = [window(10, "mousepad", 3), window(11, "gimp", 3)];
        let (plan, _, _) = settle(&windows, &shipped(), 5);
        assert_eq!(plan.moves, [(11, 10)]);
        assert_eq!(plan.focus, None);
    }

    #[test]
    fn a_window_asked_to_close_still_holds_its_workspace() {
        // 10 was killed but is still in the tree.
        let windows = [
            window(10, "ncdu", 10),
            window(11, "mousepad", 3),
            window(12, "gimp", 3),
        ];
        let (plan, _, _) = settle(&windows, &shipped(), 3);
        assert_eq!(plan.moves, [(12, 11)]);
        assert_eq!(
            launch_target(&windows, &shipped(), None, 3, &[]),
            11,
            "a launch skips it too"
        );
    }

    #[test]
    fn dialogs_go_with_their_app() {
        let windows = [focused(10, "darkwire", 5), dialog(11, "darkwire", 5)];
        let (plan, _, _) = settle(&windows, &shipped(), 5);
        assert_eq!(plan.moves, [(10, 10), (11, 10)]);
    }

    #[test]
    fn dialogs_stay_with_the_window_that_stays() {
        let windows = [
            window(10, "mousepad", 3),
            dialog(11, "mousepad", 3),
            focused(12, "gimp", 3),
        ];
        let (plan, _, _) = settle(&windows, &shipped(), 3);
        assert_eq!(plan.moves, [(12, 10)]);
    }

    #[test]
    fn a_guest_takes_its_own_dialogs_along() {
        // GIMP opened over the Editor, with its splash up. The Editor stays,
        // and the splash goes with GIMP rather than staying over the Editor.
        let windows = [
            window(10, "mousepad", 3),
            focused(11, "gimp", 3),
            dialog(12, "gimp", 3),
        ];
        let (plan, _, _) = settle(&windows, &shipped(), 3);
        assert_eq!(plan.moves, [(11, 10), (12, 10)]);
    }

    #[test]
    fn a_profile_matched_by_title_owns_no_workspace() {
        // sway reads a title once, when the window appears. It changes after.
        let mut apps = apps(&[("foot", 1), ("ncdu", 0)]);
        let notes = AppProfile {
            exec: "notes".into(),
            match_: Match {
                title: Some("Notes".into()),
                ..Match::default()
            },
            workspace: 6,
            ..AppProfile::default()
        };
        apps.apps.insert("notes".into(), notes);
        // Its window is a guest. On 6, which nobody owns, it stays.
        let windows = [focused(20, "notes-app", 6)];
        assert_eq!(plan(&windows, &apps, 6, &[]), Plan::default());
        assert_eq!(launch_target(&[], &apps, None, 5, &[]), 5);
        assert!(!owned(&apps, 6));
        assert!(!owned(&apps, 0), "a profile with no workspace owns none");
        assert_eq!(
            assign_rules(&apps),
            ["assign [app_id=\"(?i)^foot$\"] workspace number 1"],
            "no assign by title, and none for workspace 0"
        );
    }

    #[test]
    fn an_app_known_by_id_and_class_gets_both_assigns() {
        let mut apps = apps(&[("mousepad", 3)]);
        apps.apps.get_mut("mousepad").unwrap().match_.class = Some("Mousepad".into());
        assert_eq!(
            assign_rules(&apps),
            [
                "assign [app_id=\"(?i)^mousepad$\"] workspace number 3",
                "assign [class=\"(?i)^Mousepad$\"] workspace number 3",
            ]
        );
        assert_eq!(home(&apps, "Mousepad"), Some(3), "an XWayland build");
    }

    #[test]
    fn a_workspace_with_a_dialog_left_on_it_is_not_free() {
        let windows = [
            dialog(10, "gtk", 10),
            window(11, "mousepad", 3),
            window(12, "gimp", 3),
        ];
        let (plan, _, _) = settle(&windows, &shipped(), 3);
        assert_eq!(plan.moves, [(12, 11)]);
    }

    #[test]
    fn a_file_chooser_stays_with_the_app_that_asked_for_it() {
        let windows = [
            window(10, "mousepad", 3),
            focused(11, "xdg-desktop-portal-gtk", 3),
        ];
        assert_eq!(plan(&windows, &shipped(), 3, &[]), Plan::default());
    }

    #[test]
    fn with_nowhere_free_a_window_stays_put() {
        let everything = apps(
            &(1..=u8::MAX)
                .map(|n| (format!("app{n}"), n))
                .collect::<Vec<_>>(),
        );
        let windows = [window(10, "app1", 1), window(11, "gimp", 1)];
        assert_eq!(plan(&windows, &everything, 1, &[]), Plan::default());
    }

    #[test]
    fn a_launch_goes_home_unless_the_app_is_there() {
        let apps = shipped();
        assert_eq!(launch_target(&[], &apps, Some(5), 3, &[]), 5);
        let open = [window(10, "vlc", 5)];
        assert_eq!(launch_target(&open, &apps, Some(5), 3, &[]), 10);
    }

    #[test]
    fn a_guest_launch_stays_on_an_empty_unowned_workspace() {
        let apps = shipped();
        assert_eq!(launch_target(&[], &apps, None, 12, &[]), 12);
        assert_eq!(launch_target(&[], &apps, None, 5, &[]), 10, "5 is Music's");
        let open = [window(10, "ncdu", 10)];
        assert_eq!(
            launch_target(&open, &apps, None, 10, &[]),
            11,
            "10 is taken"
        );
    }

    #[test]
    fn the_whole_plan_is_one_command() {
        let plan = Plan {
            moves: vec![(10, 11), (12, 3)],
            focus: Some(10),
        };
        assert_eq!(
            plan.command().unwrap(),
            "[con_id=10] move container to workspace number 11; \
             [con_id=12] move container to workspace number 3; \
             [con_id=10] focus"
        );
        assert_eq!(Plan::default().command(), None);
    }
}
