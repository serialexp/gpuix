//! The renderer's window methods: focus, scrolling, the text selection,
//! images, fonts and the window itself. Under Node, `@gpuix/native`'s
//! `GpuixRenderer` has them as napi methods; here the embedded shim forwards
//! each one to `__gpuix.renderer.window(method, ...args)`, which [`parse`]s
//! it into a [`WindowCall`] for the runtime's [`Host`].
//!
//! `promptForPaths` is the one that answers later: the runtime keeps its
//! promise, and the host settles it through
//! [`QuickJsRuntime::settle_path_prompt`](super::QuickJsRuntime::settle_path_prompt)
//! once the picker closes.
//!
//! An app renders the first time before its window opens, and its effects
//! can call these then (`autoFocus` focuses as it mounts). [`TreeHost`]
//! answers until the window opens: it queues the calls that change
//! something, for the window to replay, and answers queries as a window that
//! hasn't painted yet would. Under Node, `init` opens the window before the
//! first render, so it answers the window's size with the size the window
//! will open at.

use rquickjs::{ArrayBuffer, Ctx, Exception, FromJs, Object, TypedArray, Value};

use crate::custom_elements::img::PixelFormat;
use crate::renderer::{
    debug_frame_overlay_mode_name, parse_debug_frame_overlay_mode_str, raw_dimension_u32,
    raw_element_id, PathPromptOptions, WindowOptions,
};
use crate::retained_tree::RetainedTree;

/// What JS changes outside the runtime: the tree the app's batches go to,
/// and the window that draws it.
pub(crate) trait Host {
    /// Run `change` on the tree the app's batches change.
    fn with_tree(&mut self, change: &mut dyn FnMut(&mut RetainedTree));

    /// Take the window options the app's renderer passed to `init`.
    fn init(&mut self, _options: &WindowOptions) {}

    /// Run one of the renderer's window methods. The result is what the
    /// napi method returns, as JSON.
    fn window(&mut self, call: WindowCall) -> Result<serde_json::Value, String>;

    /// Open the system's path picker, and settle prompt `id` with what it
    /// picked once it closes. Fails if the picker can't open.
    fn prompt_for_paths(&mut self, id: u64, options: gpui::PathPromptOptions)
        -> Result<(), String>;

    /// The calls this host queued for a window, for the host that replaces
    /// it to run.
    fn into_pending(self: Box<Self>) -> Vec<WindowCall> {
        Vec::new()
    }
}

/// One call of a window method, with its arguments checked as napi checks
/// them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WindowCall {
    WindowSize,
    WindowInsets,
    SetDebugFrameOverlay(gpui::DebugFrameOverlayMode),
    CycleDebugFrameOverlay,
    DebugFrameOverlay,
    ResetDebugFrameOverlayStats,
    DebugFrameOverlayStats,
    ActivateWindow,
    MinimizeWindow,
    ZoomWindow,
    ToggleFullscreen,
    SetWindowTitle(String),
    FocusElement(u64),
    FocusNext,
    FocusPrevious,
    FocusedElementId,
    FocusNextWithin(u64),
    FocusPreviousWithin(u64),
    SetWindowKeyEvents {
        key_down: bool,
        key_up: bool,
        event_id: u64,
    },
    Blur,
    SetWindowSelectionChange {
        enabled: bool,
        event_id: u64,
    },
    SelectedText,
    ClearSelection,
    ScrollTo {
        id: u64,
        x: f32,
        y: f32,
    },
    ScrollToItem {
        id: u64,
        index: usize,
        offset: f32,
    },
    ListScrollTop(u64),
    ScrollOffset(u64),
    ScrollIntoView(u64),
    SetImagePixels {
        id: u64,
        width: u32,
        height: u32,
        bytes: Vec<u8>,
        format: PixelFormat,
    },
    SetImage {
        id: u64,
        bytes: Vec<u8>,
    },
    RegisterFont(Vec<u8>),
    ElementBounds(u64),
    ElementsAt {
        x: f64,
        y: f64,
    },
    PaintedHighlights,
}

/// The window method `method` called with `args`. Throws a `TypeError` for
/// an unknown method or an argument napi would reject.
pub(super) fn parse<'js>(
    ctx: &Ctx<'js>,
    method: &str,
    args: &[Value<'js>],
) -> rquickjs::Result<WindowCall> {
    let args = Args { ctx, method, args };
    Ok(match method {
        "getWindowSize" => WindowCall::WindowSize,
        "getWindowInsets" => WindowCall::WindowInsets,
        "setDebugFrameOverlay" => WindowCall::SetDebugFrameOverlay(
            parse_debug_frame_overlay_mode_str(&args.string(0)?)
                .map_err(|error| args.fail(&error))?,
        ),
        "cycleDebugFrameOverlay" => WindowCall::CycleDebugFrameOverlay,
        "getDebugFrameOverlay" => WindowCall::DebugFrameOverlay,
        "resetDebugFrameOverlayStats" => WindowCall::ResetDebugFrameOverlayStats,
        "getDebugFrameOverlayStats" => WindowCall::DebugFrameOverlayStats,
        "activateWindow" => WindowCall::ActivateWindow,
        "minimizeWindow" => WindowCall::MinimizeWindow,
        "zoomWindow" => WindowCall::ZoomWindow,
        "toggleFullscreen" => WindowCall::ToggleFullscreen,
        "setWindowTitle" => WindowCall::SetWindowTitle(args.string(0)?),
        "focusElement" => WindowCall::FocusElement(args.id(0)?),
        "focusNext" => WindowCall::FocusNext,
        "focusPrevious" => WindowCall::FocusPrevious,
        "getFocusedElementId" => WindowCall::FocusedElementId,
        "focusNextWithin" => WindowCall::FocusNextWithin(args.id(0)?),
        "focusPreviousWithin" => WindowCall::FocusPreviousWithin(args.id(0)?),
        "setWindowKeyEvents" => WindowCall::SetWindowKeyEvents {
            key_down: args.bool(0)?,
            key_up: args.bool(1)?,
            event_id: args.id(2)?,
        },
        "blur" => WindowCall::Blur,
        "setWindowSelectionChange" => WindowCall::SetWindowSelectionChange {
            enabled: args.bool(0)?,
            event_id: args.id(1)?,
        },
        "getSelectedText" => WindowCall::SelectedText,
        "clearSelection" => WindowCall::ClearSelection,
        "scrollTo" => WindowCall::ScrollTo {
            id: args.id(0)?,
            x: args.number(1)? as f32,
            y: args.number(2)? as f32,
        },
        "scrollToItem" => WindowCall::ScrollToItem {
            id: args.id(0)?,
            index: args.number(1)? as usize,
            offset: args.optional_number(2)?.unwrap_or(0.0) as f32,
        },
        "getListScrollTop" => WindowCall::ListScrollTop(args.id(0)?),
        "getScrollOffset" => WindowCall::ScrollOffset(args.id(0)?),
        "scrollIntoView" => WindowCall::ScrollIntoView(args.id(0)?),
        "setImagePixels" => WindowCall::SetImagePixels {
            id: args.id(0)?,
            width: raw_dimension_u32(args.number(1)?, "width")
                .map_err(|error| args.fail(&error))?,
            height: raw_dimension_u32(args.number(2)?, "height")
                .map_err(|error| args.fail(&error))?,
            bytes: args.bytes(3)?,
            format: PixelFormat::parse(args.optional_string(4)?.as_deref())
                .map_err(|error| args.fail(&error))?,
        },
        "setImage" => WindowCall::SetImage {
            id: args.id(0)?,
            bytes: args.bytes(1)?,
        },
        "registerFont" => WindowCall::RegisterFont(args.bytes(0)?),
        "getElementBounds" => WindowCall::ElementBounds(args.id(0)?),
        "getElementsAt" => WindowCall::ElementsAt {
            x: args.number(0)?,
            y: args.number(1)?,
        },
        "getPaintedHighlights" => WindowCall::PaintedHighlights,
        _ => return Err(args.fail("the embedded renderer has no such window method")),
    })
}

/// What a path prompt settles with: the paths picked, `None` when it was
/// cancelled, or why it failed.
pub(crate) type PathPromptResult = Result<Option<Vec<String>>, String>;

/// `promptForPaths`'s options object, with its fields' types checked as
/// napi checks them. `undefined` and `null` mean the defaults.
pub(super) fn path_prompt_options<'js>(
    ctx: &Ctx<'js>,
    options: Option<Value<'js>>,
) -> rquickjs::Result<PathPromptOptions> {
    let fail = |message: &str| Exception::throw_type(ctx, &format!("promptForPaths: {message}"));
    let Some(options) = options.filter(|value| !value.is_undefined() && !value.is_null()) else {
        return Ok(PathPromptOptions::default());
    };
    let object = Object::from_value(options).map_err(|_| fail("options must be an object"))?;
    let field = |name: &str| -> rquickjs::Result<Option<Value<'js>>> {
        let value: Value = object.get(name)?;
        Ok((!value.is_undefined() && !value.is_null()).then_some(value))
    };
    let flag = |name: &str| -> rquickjs::Result<Option<bool>> {
        field(name)?
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| fail(&format!("options.{name} must be a boolean")))
            })
            .transpose()
    };
    let prompt = field("prompt")?
        .map(|value| {
            value
                .as_string()
                .ok_or_else(|| fail("options.prompt must be a string"))?
                .to_string()
        })
        .transpose()?;
    Ok(PathPromptOptions {
        files: flag("files")?,
        directories: flag("directories")?,
        multiple: flag("multiple")?,
        prompt,
    })
}

struct Args<'a, 'js> {
    ctx: &'a Ctx<'js>,
    method: &'a str,
    args: &'a [Value<'js>],
}

impl<'js> Args<'_, 'js> {
    fn fail(&self, message: &str) -> rquickjs::Error {
        Exception::throw_type(self.ctx, &format!("{}: {message}", self.method))
    }

    /// The argument at `index`, or `undefined` past the end.
    fn get(&self, index: usize) -> Option<&Value<'js>> {
        self.args
            .get(index)
            .filter(|value| !value.is_undefined() && !value.is_null())
    }

    fn number(&self, index: usize) -> rquickjs::Result<f64> {
        self.optional_number(index)?
            .ok_or_else(|| self.fail(&format!("argument {index} must be a number")))
    }

    fn optional_number(&self, index: usize) -> rquickjs::Result<Option<f64>> {
        self.get(index)
            .map(|value| {
                value
                    .as_number()
                    .ok_or_else(|| self.fail(&format!("argument {index} must be a number")))
            })
            .transpose()
    }

    fn id(&self, index: usize) -> rquickjs::Result<u64> {
        raw_element_id(self.number(index)?).map_err(|error| self.fail(&error))
    }

    fn bool(&self, index: usize) -> rquickjs::Result<bool> {
        self.get(index)
            .and_then(Value::as_bool)
            .ok_or_else(|| self.fail(&format!("argument {index} must be a boolean")))
    }

    fn string(&self, index: usize) -> rquickjs::Result<String> {
        self.optional_string(index)?
            .ok_or_else(|| self.fail(&format!("argument {index} must be a string")))
    }

    fn optional_string(&self, index: usize) -> rquickjs::Result<Option<String>> {
        self.get(index)
            .map(|value| {
                value
                    .as_string()
                    .ok_or_else(|| self.fail(&format!("argument {index} must be a string")))?
                    .to_string()
            })
            .transpose()
    }

    /// A `Uint8Array` (a Node `Buffer` is one) or an `ArrayBuffer`.
    fn bytes(&self, index: usize) -> rquickjs::Result<Vec<u8>> {
        let not_bytes = || self.fail(&format!("argument {index} must be a Uint8Array"));
        let value = self.get(index).ok_or_else(not_bytes)?;
        // SAFETY: the bytes are copied before any JS runs that could detach
        // or resize the buffer.
        if let Ok(array) = TypedArray::<u8>::from_js(self.ctx, value.clone()) {
            return unsafe { array.as_bytes() }
                .map(<[u8]>::to_vec)
                .ok_or_else(not_bytes);
        }
        let buffer = ArrayBuffer::from_js(self.ctx, value.clone()).map_err(|_| not_bytes())?;
        unsafe { buffer.as_bytes() }
            .map(<[u8]>::to_vec)
            .ok_or_else(not_bytes)
    }
}

const NO_WINDOW: &str = "the window isn't open yet";

/// The window size whoever starts the app sets, which beats the app's own
/// window options: the command line's, the size of the window a reload
/// keeps, or a test window's.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SizeOverride {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

impl SizeOverride {
    /// `options` with this size on top.
    pub(crate) fn apply(self, options: &mut WindowOptions) {
        if let Some(width) = self.width {
            options.width = Some(width);
        }
        if let Some(height) = self.height {
            options.height = Some(height);
        }
    }
}

/// The host until the window opens: it owns the tree, queues the calls that
/// change the window, and answers queries as a window that hasn't painted.
#[derive(Default)]
pub(crate) struct TreeHost {
    tree: RetainedTree,
    pending: Vec<WindowCall>,
    size: SizeOverride,
    /// What the window will open with, once the app's `init` has said.
    options: Option<WindowOptions>,
}

impl TreeHost {
    /// A host whose window will open at `size` where it's set.
    pub(crate) fn new(size: SizeOverride) -> Self {
        Self {
            size,
            ..Self::default()
        }
    }
}

impl Host for TreeHost {
    fn with_tree(&mut self, change: &mut dyn FnMut(&mut RetainedTree)) {
        change(&mut self.tree);
    }

    fn init(&mut self, options: &WindowOptions) {
        let mut options = options.clone();
        self.size.apply(&mut options);
        self.options = Some(options);
    }

    fn window(&mut self, call: WindowCall) -> Result<serde_json::Value, String> {
        use serde_json::{json, Value as Json};
        match call {
            WindowCall::WindowSize => {
                let (width, height) = self
                    .options
                    .as_ref()
                    .ok_or_else(|| NO_WINDOW.to_string())?
                    .size();
                Ok(json!({ "width": width, "height": height }))
            }
            // Like napi while a window opens. `readWindowInsets` falls back
            // until it has.
            WindowCall::WindowInsets
            | WindowCall::CycleDebugFrameOverlay
            | WindowCall::DebugFrameOverlay
            | WindowCall::DebugFrameOverlayStats => Err(NO_WINDOW.to_string()),
            WindowCall::FocusedElementId
            | WindowCall::SelectedText
            | WindowCall::ListScrollTop(_)
            | WindowCall::ScrollOffset(_)
            | WindowCall::ElementBounds(_) => Ok(Json::Null),
            WindowCall::ElementsAt { .. } | WindowCall::PaintedHighlights => Ok(json!([])),
            // Nothing is selected before the window opens.
            WindowCall::ClearSelection => Ok(Json::Null),
            WindowCall::SetDebugFrameOverlay(mode) => {
                self.pending.push(call);
                Ok(json!(debug_frame_overlay_mode_name(mode)))
            }
            call => {
                self.pending.push(call);
                Ok(Json::Null)
            }
        }
    }

    fn prompt_for_paths(
        &mut self,
        _id: u64,
        _options: gpui::PathPromptOptions,
    ) -> Result<(), String> {
        Err(NO_WINDOW.to_string())
    }

    fn into_pending(self: Box<Self>) -> Vec<WindowCall> {
        self.pending
    }
}
