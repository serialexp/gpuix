//! The application an embedded script defines: its windows, the commands it
//! sends them, and its focus requests.
//!
//! A script entry either returns one render function, which gets a single
//! window, or creates an application and defines named windows on it. The
//! registry is generic over `R`, the engine's render callback, and leaves
//! error wording to each engine, so messages can name its own API.

use std::collections::VecDeque;

/// One window's settings, as the host opens it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WindowOptions {
    pub id: String,
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub open: bool,
    pub focus: bool,
    pub reposition: bool,
}

impl WindowOptions {
    /// The window an entry that returns a single render function gets.
    pub(crate) fn single(id: &str) -> Self {
        Self {
            id: id.to_string(),
            title: "GPUIX".to_string(),
            width: 800.0,
            height: 600.0,
            open: true,
            focus: true,
            reposition: false,
        }
    }
}

/// A window definition as a script wrote it, before defaults and checks.
#[derive(Clone, Debug, Default)]
pub(crate) struct WindowSpec {
    pub id: String,
    /// Defaults to the id.
    pub title: Option<String>,
    /// Defaults to 800.
    pub width: Option<f32>,
    /// Defaults to 600.
    pub height: Option<f32>,
    /// Whether the window opens with the application. Defaults to true.
    pub open: Option<bool>,
    /// Whether the window takes focus when it opens. Defaults to true.
    pub focus: Option<bool>,
    /// Whether the window may move to fit the screen. Defaults to false.
    pub reposition: Option<bool>,
}

impl WindowSpec {
    pub(crate) fn into_options(self) -> Result<WindowOptions, String> {
        if self.id.trim().is_empty() {
            return Err("window id cannot be empty".to_string());
        }
        Ok(WindowOptions {
            title: self.title.unwrap_or_else(|| self.id.clone()),
            width: positive_dimension(self.width, 800.0, "width")?,
            height: positive_dimension(self.height, 600.0, "height")?,
            open: self.open.unwrap_or(true),
            focus: self.focus.unwrap_or(true),
            reposition: self.reposition.unwrap_or(false),
            id: self.id,
        })
    }
}

fn positive_dimension(value: Option<f32>, default: f32, name: &str) -> Result<f32, String> {
    let value = value.unwrap_or(default);
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("window {name} must be greater than zero"));
    }
    Ok(value)
}

/// What a script asks the host to do with one of its windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AppCommand {
    Open(String),
    Close(String),
    Focus(String),
    SetTitle { id: String, title: String },
}

impl AppCommand {
    fn window_id(&self) -> &str {
        match self {
            Self::Open(id) | Self::Close(id) | Self::Focus(id) | Self::SetTitle { id, .. } => id,
        }
    }
}

/// A script's request to focus an element in one of its windows. The last
/// request made during an event wins.
pub(crate) struct FocusRequest {
    pub root_id: String,
    pub element_id: u64,
}

#[derive(Clone)]
pub(crate) struct WindowDefinition<R> {
    pub options: WindowOptions,
    pub render: R,
}

/// Why the registry refused a call. Each engine words these itself.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RegistryError {
    /// The entry created an application before.
    AlreadyCreated,
    /// A window was defined before the entry created an application.
    NotCreated,
    DuplicateWindow(String),
    UnknownWindow(String),
}

/// The windows and pending commands of the application the current entry
/// defines. Cloned to roll back a failed reload.
#[derive(Clone)]
pub(crate) struct ApplicationRegistry<R> {
    created: bool,
    definitions: Vec<WindowDefinition<R>>,
    commands: VecDeque<AppCommand>,
}

impl<R> Default for ApplicationRegistry<R> {
    fn default() -> Self {
        Self {
            created: false,
            definitions: Vec::new(),
            commands: VecDeque::new(),
        }
    }
}

impl<R> ApplicationRegistry<R> {
    /// Forget the previous entry's application before running an entry.
    pub(crate) fn begin_entry(&mut self) {
        self.created = false;
        self.definitions.clear();
        self.commands.clear();
    }

    /// Whether the entry created an application.
    pub(crate) fn is_created(&self) -> bool {
        self.created
    }

    /// Create the entry's application. An entry creates at most one.
    pub(crate) fn create(&mut self) -> Result<(), RegistryError> {
        if self.created {
            return Err(RegistryError::AlreadyCreated);
        }
        self.created = true;
        Ok(())
    }

    pub(crate) fn define(&mut self, definition: WindowDefinition<R>) -> Result<(), RegistryError> {
        if !self.created {
            return Err(RegistryError::NotCreated);
        }
        if self.is_defined(&definition.options.id) {
            return Err(RegistryError::DuplicateWindow(definition.options.id));
        }
        self.definitions.push(definition);
        Ok(())
    }

    pub(crate) fn is_defined(&self, id: &str) -> bool {
        self.definitions
            .iter()
            .any(|definition| definition.options.id == id)
    }

    /// Queue a command for a defined window.
    pub(crate) fn queue(&mut self, command: AppCommand) -> Result<(), RegistryError> {
        if !self.is_defined(command.window_id()) {
            return Err(RegistryError::UnknownWindow(
                command.window_id().to_string(),
            ));
        }
        self.commands.push_back(command);
        Ok(())
    }

    /// The commands queued since the last call, in order.
    pub(crate) fn take_commands(&mut self) -> Vec<AppCommand> {
        self.commands.drain(..).collect()
    }

    pub(crate) fn definitions(&self) -> &[WindowDefinition<R>] {
        &self.definitions
    }

    pub(crate) fn window_options(&self) -> Vec<WindowOptions> {
        self.definitions
            .iter()
            .map(|definition| definition.options.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str) -> WindowDefinition<()> {
        WindowDefinition {
            options: WindowSpec {
                id: id.to_string(),
                ..WindowSpec::default()
            }
            .into_options()
            .unwrap(),
            render: (),
        }
    }

    #[test]
    fn window_specs_get_defaults_and_are_checked() {
        let options = WindowSpec {
            id: "main".to_string(),
            ..WindowSpec::default()
        }
        .into_options()
        .unwrap();
        assert_eq!(
            options,
            WindowOptions {
                id: "main".to_string(),
                title: "main".to_string(),
                width: 800.0,
                height: 600.0,
                open: true,
                focus: true,
                reposition: false,
            }
        );
        let error = |spec: WindowSpec| spec.into_options().unwrap_err();
        assert_eq!(error(WindowSpec::default()), "window id cannot be empty");
        assert_eq!(
            error(WindowSpec {
                id: "main".to_string(),
                width: Some(0.0),
                ..WindowSpec::default()
            }),
            "window width must be greater than zero"
        );
        assert_eq!(
            error(WindowSpec {
                id: "main".to_string(),
                height: Some(f32::NAN),
                ..WindowSpec::default()
            }),
            "window height must be greater than zero"
        );
    }

    #[test]
    fn the_registry_holds_one_application_per_entry() {
        let mut registry = ApplicationRegistry::default();
        assert_eq!(
            registry.define(window("main")),
            Err(RegistryError::NotCreated)
        );
        registry.create().unwrap();
        assert_eq!(registry.create(), Err(RegistryError::AlreadyCreated));
        registry.define(window("main")).unwrap();
        assert_eq!(
            registry.define(window("main")),
            Err(RegistryError::DuplicateWindow("main".to_string()))
        );
        registry.define(window("inspector")).unwrap();
        let ids: Vec<String> = registry
            .window_options()
            .into_iter()
            .map(|options| options.id)
            .collect();
        assert_eq!(ids, ["main", "inspector"]);

        registry.begin_entry();
        assert!(!registry.is_created());
        assert!(registry.definitions().is_empty());
    }

    #[test]
    fn commands_need_a_defined_window_and_drain_in_order() {
        let mut registry = ApplicationRegistry::default();
        registry.create().unwrap();
        registry.define(window("main")).unwrap();
        assert_eq!(
            registry.queue(AppCommand::Open("missing".to_string())),
            Err(RegistryError::UnknownWindow("missing".to_string()))
        );
        registry
            .queue(AppCommand::Focus("main".to_string()))
            .unwrap();
        registry
            .queue(AppCommand::SetTitle {
                id: "main".to_string(),
                title: "Main".to_string(),
            })
            .unwrap();
        assert_eq!(
            registry.take_commands(),
            [
                AppCommand::Focus("main".to_string()),
                AppCommand::SetTitle {
                    id: "main".to_string(),
                    title: "Main".to_string()
                }
            ]
        );
        assert!(registry.take_commands().is_empty());
    }
}
