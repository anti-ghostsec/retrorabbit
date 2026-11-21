//! Glue to the Java side (MainActivity.kt / TvPlayer.kt): the JVM, the activity,
//! the app's private folder, media probing and the repaint hook.

use jni::objects::{GlobalRef, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM};
use std::path::PathBuf;
use std::sync::OnceLock;
use winit::platform::android::activity::AndroidApp;

struct Jvm {
    vm: JavaVM,
    activity: GlobalRef,
}

static JVM: OnceLock<Jvm> = OnceLock::new();
static DATA_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
static EGUI: OnceLock<eframe::egui::Context> = OnceLock::new();

pub fn init(app: &AndroidApp) {
    let _ = DATA_DIR.set(app.internal_data_path());
    // SAFETY: android-activity hands us the process's JavaVM and a global ref to the activity.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) };
    let Ok(vm) = vm else { return };
    let activity = {
        let Ok(env) = vm.attach_current_thread_permanently() else { return };
        // SAFETY: activity_as_ptr is a valid jobject (a global ref) for the app's lifetime.
        let obj = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let Ok(g) = env.new_global_ref(obj) else { return };
        g
    };
    let _ = JVM.set(Jvm { vm, activity });
}

/// Run `f` with a JNI env attached to this thread and the activity object.
pub fn with<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Option<R> {
    let j = JVM.get()?;
    let mut env = j.vm.attach_current_thread().ok()?;
    let r = f(&mut env, j.activity.as_obj());
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    r.ok()
}

pub fn data_dir() -> Option<PathBuf> {
    DATA_DIR.get().cloned().flatten()
}

/// The TV's own video folders (shared storage), if it has any.
pub fn video_folders() -> Vec<String> {
    ["/storage/emulated/0/Movies", "/storage/emulated/0/Download", "/storage/emulated/0/DCIM"]
        .iter()
        .filter(|p| std::path::Path::new(p).is_dir())
        .map(|p| p.to_string())
        .collect()
}

/// ffprobe's job on Android: MediaMetadataRetriever (MainActivity.probe) answers
/// with the same JSON shape ffprobe prints, so library.rs reads it unchanged.
pub struct Probe {
    pub stdout: Vec<u8>,
}

pub fn probe(path: &std::path::Path) -> Option<Probe> {
    let path = path.to_string_lossy().to_string();
    with(|env, act| {
        let p = env.new_string(&path)?;
        let out = env.call_method(act, "probe", "(Ljava/lang/String;)Ljava/lang/String;", &[JValue::Object(&p)])?.l()?;
        if out.is_null() {
            return Ok(None);
        }
        let s: String = env.get_string(&JString::from(out))?.into();
        Ok(Some(Probe { stdout: s.into_bytes() }))
    })
    .flatten()
}

/// The activity tells us when storage permission was just granted (rescan then).
pub fn take_permission_granted() -> bool {
    with(|env, act| env.call_method(act, "takePermissionGranted", "()Z", &[])?.z()).unwrap_or(false)
}

/// Pixels covered by a notch at each edge: left, top, right, bottom.
pub fn safe_insets() -> [i32; 4] {
    with(|env, act| {
        let arr = env.call_method(act, "safeInsets", "()[I", &[])?.l()?;
        let arr = jni::objects::JIntArray::from(arr);
        let mut v = [0i32; 4];
        env.get_int_array_region(&arr, 0, &mut v)?;
        Ok(v)
    })
    .unwrap_or([0; 4])
}

pub fn set_egui(ctx: &eframe::egui::Context) {
    let _ = EGUI.set(ctx.clone());
}

pub fn repaint() {
    if let Some(c) = EGUI.get() {
        c.request_repaint();
    }
}

/// Called by TvPlayer.kt when a new video frame or a state change arrives.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_retrorabbit_tv_TvPlayer_nativeWake(_env: JNIEnv, _class: JObject) {
    repaint();
}
