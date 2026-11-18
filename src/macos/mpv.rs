//! The bit of libmpv's C API RetroRabbit uses on macOS, loaded at run time so the
//! build doesn't need libmpv. macOS can't show another process's window inside
//! ours, so mpv runs in-process and renders into an OpenGL texture (render API).

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::OnceLock;

pub type Handle = *mut c_void;

#[repr(C)]
pub struct Event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct EventEndFile {
    pub reason: c_int,
    pub error: c_int,
}

#[repr(C)]
pub struct RenderParam {
    pub kind: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct OpenGlInitParams {
    pub get_proc_address: unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct OpenGlFbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_START_FILE: c_int = 6;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_PLAYBACK_RESTART: c_int = 21;
pub const END_FILE_EOF: c_int = 0;
pub const END_FILE_ERROR: c_int = 4;

pub const RENDER_PARAM_INVALID: c_int = 0;
pub const RENDER_PARAM_API_TYPE: c_int = 1;
pub const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const RENDER_PARAM_FLIP_Y: c_int = 4;
pub const RENDER_UPDATE_FRAME: u64 = 1;

type UpdateCb = unsafe extern "C" fn(*mut c_void);

pub struct Api {
    _lib: libloading::Library,
    pub create: unsafe extern "C" fn() -> Handle,
    pub initialize: unsafe extern "C" fn(Handle) -> c_int,
    pub set_option_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    pub set_property_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    pub command: unsafe extern "C" fn(Handle, *mut *const c_char) -> c_int,
    pub wait_event: unsafe extern "C" fn(Handle, f64) -> *mut Event,
    pub error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    pub render_context_create: unsafe extern "C" fn(*mut Handle, Handle, *mut RenderParam) -> c_int,
    pub render_context_set_update_callback: unsafe extern "C" fn(Handle, Option<UpdateCb>, *mut c_void),
    pub render_context_update: unsafe extern "C" fn(Handle) -> u64,
    pub render_context_render: unsafe extern "C" fn(Handle, *mut RenderParam) -> c_int,
}

// SAFETY: plain C function pointers; libmpv's client API is thread-safe.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn candidates() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        // RetroRabbit.app/Contents/MacOS/retrorabbit → Contents/Frameworks
        v.push(dir.join("../Frameworks/libmpv.2.dylib"));
        v.push(dir.join("libmpv.2.dylib"));
    }
    for d in ["/opt/homebrew/lib", "/usr/local/lib", "/opt/local/lib"] {
        v.push(std::path::Path::new(d).join("libmpv.2.dylib"));
    }
    v.push("libmpv.2.dylib".into());
    v
}

fn load() -> Result<Api, String> {
    let mut last = String::from("libmpv not found");
    for path in candidates() {
        // SAFETY: loading libmpv runs its (trusted) initialisers.
        match unsafe { libloading::Library::new(&path) } {
            Ok(lib) => return bind(lib),
            Err(e) => last = format!("{}: {e}", path.display()),
        }
    }
    Err(format!("Video needs libmpv (brew install mpv). {last}"))
}

fn sym<T: Copy>(lib: &libloading::Library, name: &str) -> Result<T, String> {
    let n = format!("{name}\0");
    // SAFETY: each caller's T matches libmpv's client.h / render.h signature.
    unsafe { lib.get::<T>(n.as_bytes()).map(|s| *s).map_err(|e| format!("libmpv: {e}")) }
}

fn bind(lib: libloading::Library) -> Result<Api, String> {
    Ok(Api {
        create: sym(&lib, "mpv_create")?,
        initialize: sym(&lib, "mpv_initialize")?,
        set_option_string: sym(&lib, "mpv_set_option_string")?,
        set_property_string: sym(&lib, "mpv_set_property_string")?,
        command: sym(&lib, "mpv_command")?,
        wait_event: sym(&lib, "mpv_wait_event")?,
        error_string: sym(&lib, "mpv_error_string")?,
        render_context_create: sym(&lib, "mpv_render_context_create")?,
        render_context_set_update_callback: sym(&lib, "mpv_render_context_set_update_callback")?,
        render_context_update: sym(&lib, "mpv_render_context_update")?,
        render_context_render: sym(&lib, "mpv_render_context_render")?,
        _lib: lib,
    })
}

pub fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(|e| e.clone())
}

/// An mpv core. Never destroyed: Player sends `quit` and the process ends soon after.
#[derive(Clone, Copy)]
pub struct Mpv(pub Handle);

// SAFETY: libmpv handles may be used from any thread.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

fn cs(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl Mpv {
    /// A new core with `options` (`name=value`, as on mpv's command line, minus `--`).
    pub fn new(options: &[String]) -> Result<Self, String> {
        let api = api()?;
        // SAFETY: libmpv calls with valid, NUL-terminated strings.
        unsafe {
            let h = (api.create)();
            if h.is_null() {
                return Err("mpv_create failed".into());
            }
            for o in options {
                let (k, v) = o.trim_start_matches("--").split_once('=').unwrap_or((o.trim_start_matches("--"), "yes"));
                (api.set_option_string)(h, cs(k).as_ptr(), cs(v).as_ptr());
            }
            let r = (api.initialize)(h);
            if r < 0 {
                return Err(format!("mpv_initialize: {}", error_string(r)));
            }
            Ok(Mpv(h))
        }
    }

    pub fn command(&self, args: &[&str]) {
        let Ok(api) = api() else { return };
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: NULL-terminated array of valid strings, alive for the call.
        unsafe {
            (api.command)(self.0, ptrs.as_mut_ptr());
        }
    }

    pub fn set(&self, name: &str, value: &str) {
        let Ok(api) = api() else { return };
        // SAFETY: valid strings, alive for the call.
        unsafe {
            (api.set_property_string)(self.0, cs(name).as_ptr(), cs(value).as_ptr());
        }
    }
}

pub fn error_string(code: c_int) -> String {
    let Ok(api) = api() else { return "error".into() };
    // SAFETY: mpv_error_string returns a static string.
    unsafe {
        let p = (api.error_string)(code);
        if p.is_null() { "error".into() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}
