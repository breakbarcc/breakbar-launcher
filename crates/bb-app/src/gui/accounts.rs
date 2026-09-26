//! Adding, editing, moving and deleting accounts, and what hangs off an account (shortcut, profile folder).

use super::{
    Account, AccountId, AccountState, App, CompanionId, CompanionToggle, ComponentHandle, Config,
    Draft, EditorData, LoginState, MainWindow, Messages, Model, Provider, Rc, RefCell, Scope,
    SharedString, ToastKind, Trigger, account_row, apply_account, idle_state, insert_row,
    is_active, move_row, offer_login_setup, push_toast, refresh, remove_row, row, ui, update_row,
};

/// Adds `account` to the config and the list, at `index` (the end if `None`).
pub(super) fn insert_account(
    window: &MainWindow,
    app: &RefCell<App>,
    account: Account,
    index: Option<usize>,
) {
    let mut app_ref = app.borrow_mut();
    let row = account_row(&account, &app_ref.config.companions);
    let index = index
        .unwrap_or(app_ref.config.accounts.len())
        .min(app_ref.config.accounts.len());
    app_ref.config.accounts.insert(index, account);
    app_ref.save(window);
    drop(app_ref);

    insert_row(window, index, row);
    refresh(window);
}

/// Opens the editor for account `id`, or for a new account if `None`.
pub(super) fn open_editor(window: &MainWindow, app: &RefCell<App>, id: Option<AccountId>) {
    let mut app_ref = app.borrow_mut();
    let account = match id {
        Some(id) => match app_ref.account(id) {
            Some(account) => Some(account.clone()),
            None => return,
        },
        None => None,
    };
    let draft = Draft {
        id,
        companions: account
            .as_ref()
            .map(|account| account.companions.clone())
            .unwrap_or_default(),
    };
    let active = id
        .and_then(|id| row(window, id))
        .is_some_and(|row| is_active(row.state));
    window.set_editor(editor_data(
        &app_ref.config,
        account.as_ref(),
        &draft,
        active,
    ));
    app_ref.draft = Some(draft);
    window.set_page(ui::Page::Editor);
}

pub(super) fn editor_data(
    config: &Config,
    account: Option<&Account>,
    draft: &Draft,
    active: bool,
) -> EditorData {
    let steam = account.is_some_and(|account| account.provider == Provider::Steam);
    let login = match account {
        _ if steam => LoginState::Steam,
        Some(account) if bb_store::is_set_up(account.id) => LoginState::SetUp,
        _ => LoginState::Missing,
    };
    let companions: Vec<CompanionToggle> = config
        .companions
        .iter()
        .map(|app| CompanionToggle {
            id: app.id.0 as i32,
            name: app.name.as_str().into(),
            per_client: app.scope == Scope::PerClient,
            after_game_start: app.start_when == Trigger::WindowShown,
            enabled: draft.companions.contains(&app.id),
        })
        .collect();
    EditorData {
        id: draft.id.map_or(0, |id| id.0 as i32),
        name: account.map_or_else(SharedString::new, |account| account.name.as_str().into()),
        steam,
        args: account.map_or_else(SharedString::new, |account| {
            account.extra_args.as_str().into()
        }),
        login,
        active,
        companions: Rc::new(slint::VecModel::from(companions)).into(),
    }
}

pub(super) fn editor_toggle_companion(
    window: &MainWindow,
    app: &RefCell<App>,
    companion: CompanionId,
) {
    let mut app_ref = app.borrow_mut();
    let Some(draft) = app_ref.draft.as_mut() else {
        return;
    };
    if let Some(position) = draft.companions.iter().position(|&id| id == companion) {
        draft.companions.remove(position);
    } else {
        draft.companions.push(companion);
    }
    let enabled = draft.companions.contains(&companion);

    let editor = window.get_editor();
    let toggles: Vec<CompanionToggle> = editor
        .companions
        .iter()
        .map(|mut toggle| {
            if toggle.id == companion.0 as i32 {
                toggle.enabled = enabled;
            }
            toggle
        })
        .collect();
    window.set_editor(EditorData {
        companions: Rc::new(slint::VecModel::from(toggles)).into(),
        ..editor
    });
}

/// Saves the editor: creates the new account or updates the edited one.
pub(super) fn editor_save(
    window: &MainWindow,
    app: &RefCell<App>,
    name: &str,
    steam: bool,
    args: &str,
) {
    if name.is_empty() {
        return;
    }
    let Some(draft) = app.borrow_mut().draft.take() else {
        return;
    };
    let provider = if steam {
        Provider::Steam
    } else {
        Provider::ArenaNet
    };

    match draft.id {
        None => {
            let mut account = Account::new(app.borrow().next_account_id(), name);
            account.provider = provider;
            args.clone_into(&mut account.extra_args);
            account.companions = draft.companions;
            let (id, name) = (account.id, account.name.clone());
            insert_account(window, app, account, None);
            offer_login_setup(window, id, &name, steam);
        }
        Some(id) => {
            let mut app_ref = app.borrow_mut();
            let Some(index) = app_ref.account_index(id) else {
                return;
            };
            let account = &mut app_ref.config.accounts[index];
            name.clone_into(&mut account.name);
            account.provider = provider;
            args.clone_into(&mut account.extra_args);
            account.companions = draft.companions;
            let account = account.clone();
            app_ref.save(window);
            let companions = app_ref.config.companions.clone();
            drop(app_ref);

            update_row(window, id, |row| {
                apply_account(row, &account, &companions);
                // The platform decides whether a missing login matters.
                if matches!(row.state, AccountState::Idle | AccountState::NeedsLogin) {
                    row.state = idle_state(id, row.steam);
                }
            });
        }
    }
    window.set_page(ui::Page::Accounts);
}

/// Copies an account's settings (not its login) into a new account right below it.
pub(super) fn duplicate_account(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let (copy, index) = {
        let app_ref = app.borrow();
        let (Some(source), Some(index)) = (app_ref.account(id), app_ref.account_index(id)) else {
            return;
        };
        let name = window
            .global::<Messages>()
            .invoke_copy_name(source.name.as_str().into());
        let mut copy = Account::new(app_ref.next_account_id(), name.as_str());
        copy.provider = source.provider;
        copy.extra_args.clone_from(&source.extra_args);
        copy.companions.clone_from(&source.companions);
        (copy, index + 1)
    };
    insert_account(window, app, copy, Some(index));
}

/// Moves account `id` to list position `index` (clamped to the list).
pub(super) fn move_account(window: &MainWindow, app: &RefCell<App>, id: AccountId, index: i32) {
    let mut app_ref = app.borrow_mut();
    let Some(from) = app_ref.account_index(id) else {
        return;
    };
    let last = app_ref.config.accounts.len() - 1;
    let to = (index.max(0) as usize).min(last);
    if from == to {
        return;
    }
    let account = app_ref.config.accounts.remove(from);
    app_ref.config.accounts.insert(to, account);
    app_ref.save(window);
    drop(app_ref);

    move_row(window, id, to);
}

/// Deletes the account and, for good, its profile folder with the saved login. The UI has asked
/// the user first.
pub(super) fn delete_account(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let messages = window.global::<Messages>();
    let Some(row) = row(window, id) else {
        return;
    };
    if is_active(row.state) {
        push_toast(
            window,
            ToastKind::Warning,
            messages.invoke_delete_running(row.name),
            SharedString::new(),
        );
        return;
    }

    let mut app_ref = app.borrow_mut();
    let Some(index) = app_ref.account_index(id) else {
        return;
    };
    app_ref.config.accounts.remove(index);
    app_ref.save(window);
    if app_ref
        .draft
        .as_ref()
        .is_some_and(|draft| draft.id == Some(id))
    {
        app_ref.draft = None;
        window.set_page(ui::Page::Accounts);
    }
    drop(app_ref);

    remove_row(window, id);
    refresh(window);

    match bb_store::delete_profile(id) {
        Ok(()) => push_toast(
            window,
            ToastKind::Success,
            messages.invoke_deleted_title(row.name),
            SharedString::new(),
        ),
        Err(error) => push_toast(
            window,
            ToastKind::Error,
            messages.invoke_delete_failed_title(),
            error.to_string().into(),
        ),
    }
}

/// Puts a shortcut on the desktop that starts the account without opening the launcher.
pub(super) fn create_shortcut(window: &MainWindow, app: &RefCell<App>, id: AccountId) {
    let messages = window.global::<Messages>();
    let Some(name) = app.borrow().account(id).map(|account| account.name.clone()) else {
        return;
    };
    let result = (|| -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let desktop = bb_win::shortcut::desktop_dir().map_err(|error| error.to_string())?;
        let file = format!("{} (Breakbar).lnk", file_name_safe(&name));
        let description = messages.invoke_shortcut_description(name.as_str().into());
        bb_win::shortcut::create(
            &desktop.join(&file),
            &bb_win::shortcut::Shortcut {
                target: &exe,
                arguments: &format!("--launch-id {}", id.0),
                working_dir: exe.parent().unwrap_or(&desktop),
                description: &description,
                icon: &exe,
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(file)
    })();
    match result {
        Ok(file) => push_toast(
            window,
            ToastKind::Success,
            messages.invoke_shortcut_created_title(),
            messages.invoke_shortcut_created(file.into()),
        ),
        Err(error) => push_toast(
            window,
            ToastKind::Error,
            messages.invoke_shortcut_failed_title(),
            error.into(),
        ),
    }
}

/// Replaces characters Windows doesn't allow in file names.
pub(super) fn file_name_safe(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_owned()
}

pub(super) fn open_profile_folder(window: &MainWindow, id: AccountId) {
    let result = bb_store::ensure_profile_dir(id)
        .map_err(|error| error.to_string())
        .and_then(|dir| {
            std::process::Command::new("explorer.exe")
                .arg(dir)
                .spawn()
                .map(drop)
                .map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        push_toast(
            window,
            ToastKind::Error,
            window.global::<Messages>().invoke_folder_failed_title(),
            error.into(),
        );
    }
}

/// Connects the window's callbacks for this area.
pub(super) fn wire(window: &MainWindow, app: &Rc<RefCell<App>>) {
    window.on_edit_account({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                let id = (id != 0).then_some(AccountId(id as u32));
                open_editor(&window, &app, id);
            }
        }
    });

    window.on_editor_toggle_companion({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |companion| {
            if let Some(window) = weak.upgrade() {
                editor_toggle_companion(&window, &app, CompanionId(companion as u32));
            }
        }
    });

    window.on_editor_save({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |name, steam, args| {
            if let Some(window) = weak.upgrade() {
                editor_save(&window, &app, name.trim(), steam, args.trim());
            }
        }
    });

    window.on_duplicate_account({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                duplicate_account(&window, &app, AccountId(id as u32));
            }
        }
    });

    window.on_move_account({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |id, index| {
            if let Some(window) = weak.upgrade() {
                move_account(&window, &app, AccountId(id as u32), index);
            }
        }
    });

    window.on_delete_account({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                delete_account(&window, &app, AccountId(id as u32));
            }
        }
    });

    window.on_create_shortcut({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                create_shortcut(&window, &app, AccountId(id as u32));
            }
        }
    });

    window.on_open_profile_folder({
        let weak = window.as_weak();
        move |id| {
            if let Some(window) = weak.upgrade() {
                open_profile_folder(&window, AccountId(id as u32));
            }
        }
    });

    window.on_setup_create_account({
        let app = Rc::clone(app);
        let weak = window.as_weak();
        move |name, steam| {
            if let Some(window) = weak.upgrade() {
                let mut account = Account::new(app.borrow().next_account_id(), name.trim());
                if steam {
                    account.provider = Provider::Steam;
                }
                let (id, name) = (account.id, account.name.clone());
                insert_account(&window, &app, account, None);
                window.set_page(ui::Page::Accounts);
                offer_login_setup(&window, id, &name, steam);
            }
        }
    });
}
