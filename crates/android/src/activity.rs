//! Calls into the Java activity through JNI: the share sheet, the
//! microphone permission, leaving the app and whether its window shows.

use jni::JNIEnv;
use jni::objects::{JObject, JValue};
use winit::platform::android::activity::AndroidApp;

const RECORD_AUDIO: &str = "android.permission.RECORD_AUDIO";

/// Run `f` with the activity on this thread's JNI environment. `None` if
/// that fails; a Java exception is logged and cleared.
pub(crate) fn with_activity<R>(
    app: &AndroidApp,
    f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>,
) -> Option<R> {
    // SAFETY: both pointers come from android-activity and stay valid for as
    // long as the app runs; the activity is a global reference.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let result = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result.inspect_err(|err| log::warn!("JNI call failed: {err}")).ok()
}

/// The Android side of [`grooph_app::Host`].
#[derive(Clone)]
pub(crate) struct Activity {
    pub(crate) app: AndroidApp,
}

impl grooph_app::Host for Activity {
    /// `Intent.createChooser(new Intent(ACTION_SEND) with the link as text)`.
    fn share(&self, url: &str) -> bool {
        with_activity(&self.app, |env, activity| {
            let action = env.new_string("android.intent.action.SEND")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[(&action).into()],
            )?;
            let mime = env.new_string("text/plain")?;
            env.call_method(
                &intent,
                "setType",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[(&mime).into()],
            )?;
            let key = env.new_string("android.intent.extra.TEXT")?;
            let text = env.new_string(url)?;
            env.call_method(
                &intent,
                "putExtra",
                "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                &[(&key).into(), (&text).into()],
            )?;
            let chooser = env
                .call_static_method(
                    "android/content/Intent",
                    "createChooser",
                    "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                    &[(&intent).into(), JValue::Object(&JObject::null())],
                )?
                .l()?;
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[(&chooser).into()],
            )?;
            Ok(())
        })
        .is_some()
    }

    fn mic_permitted(&self) -> bool {
        with_activity(&self.app, |env, activity| {
            let permission = env.new_string(RECORD_AUDIO)?;
            let result = env
                .call_method(
                    activity,
                    "checkSelfPermission",
                    "(Ljava/lang/String;)I",
                    &[(&permission).into()],
                )?
                .i()?;
            // PackageManager.PERMISSION_GRANTED
            Ok(result == 0)
        })
        .unwrap_or(false)
    }

    /// `activity.requestPermissions(new String[] { RECORD_AUDIO }, 1)`.
    fn request_mic(&self) {
        with_activity(&self.app, |env, activity| {
            let permission = env.new_string(RECORD_AUDIO)?;
            let permissions = env.new_object_array(1, "java/lang/String", &permission)?;
            env.call_method(
                activity,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[(&permissions).into(), 1.into()],
            )?;
            Ok(())
        });
    }
}

impl Activity {
    /// Send the app to the background, as the back button does on the
    /// first screen of other apps (`moveTaskToBack`).
    pub(crate) fn move_to_back(&self) {
        with_activity(&self.app, |env, activity| {
            env.call_method(activity, "moveTaskToBack", "(Z)Z", &[true.into()])?;
            Ok(())
        });
    }

    /// Open or close the on-screen keyboard through the InputMethodManager.
    /// NativeActivity's own call (`ANativeActivity_showSoftInput`) does
    /// nothing on current Android versions.
    pub(crate) fn soft_keyboard(&self, open: bool) {
        let done = with_activity(&self.app, |env, activity| {
            let service = env.new_string("input_method")?;
            let imm = env
                .call_method(
                    activity,
                    "getSystemService",
                    "(Ljava/lang/String;)Ljava/lang/Object;",
                    &[(&service).into()],
                )?
                .l()?;
            // The view with the focus is NativeActivity's content view, the
            // one the input method serves.
            let mut view =
                env.call_method(activity, "getCurrentFocus", "()Landroid/view/View;", &[])?.l()?;
            if view.is_null() {
                view = decor_view(env, activity)?;
            }
            if !open {
                let token =
                    env.call_method(&view, "getWindowToken", "()Landroid/os/IBinder;", &[])?.l()?;
                return env
                    .call_method(
                        &imm,
                        "hideSoftInputFromWindow",
                        "(Landroid/os/IBinder;I)Z",
                        &[(&token).into(), 0.into()],
                    )?
                    .z();
            }
            let shown = env
                .call_method(
                    &imm,
                    "showSoftInput",
                    "(Landroid/view/View;I)Z",
                    &[(&view).into(), 0.into()],
                )?
                .z()?;
            if !shown {
                // The view is not a text editor, so the input method may
                // turn the request down; toggling still opens it.
                // InputMethodManager.SHOW_FORCED
                env.call_method(&imm, "toggleSoftInput", "(II)V", &[2.into(), 0.into()])?;
            }
            Ok(shown)
        });
        log::info!("Keyboard {}: {done:?}", if open { "open" } else { "close" });
    }

    /// Whether the activity's window is hidden (`View.getWindowVisibility()`
    /// is not `VISIBLE`): the app went to the background.
    pub(crate) fn window_hidden(&self) -> bool {
        with_activity(&self.app, |env, activity| {
            let decor = decor_view(env, activity)?;
            let visibility = env.call_method(&decor, "getWindowVisibility", "()I", &[])?.i()?;
            // View.VISIBLE
            Ok(visibility != 0)
        })
        .unwrap_or(false)
    }
}

/// `activity.getWindow().getDecorView()`.
pub(crate) fn decor_view<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject,
) -> jni::errors::Result<JObject<'local>> {
    let window = env.call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
    env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()
}
