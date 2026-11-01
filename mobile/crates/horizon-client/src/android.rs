//! Things only Android can do, through the activity's JNI: open a link in the browser,
//! put text on the clipboard and start the app again.
#![cfg(target_os = "android")]
// JNI needs raw pointers from the activity.
#![allow(unsafe_code)]

use jni::objects::{JObject, JValue};

fn with_activity<T>(
    f: impl FnOnce(&mut jni::JNIEnv, &JObject) -> Result<T, jni::errors::Error>,
) -> Result<T, String> {
    let app = crate::ANDROID_APP
        .get()
        .ok_or("the Android activity is not set")?;
    // SAFETY: the pointers come from AndroidApp and stay valid as long as the activity.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }
        .map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
    let result = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    result.map_err(|e| e.to_string())
}

/// Opens `uri` in the system browser.
pub fn open_uri(uri: &str) -> Result<(), String> {
    with_activity(|env, activity| {
        let text = env.new_string(uri)?;
        let parsed = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&text)],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&parsed)],
        )?;
        // FLAG_ACTIVITY_NEW_TASK
        env.call_method(
            &intent,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(0x1000_0000)],
        )?;
        env.call_method(
            activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    })
}

/// Puts `text` on the system clipboard.
pub fn copy_text(text: &str) -> Result<(), String> {
    with_activity(|env, activity| {
        let name = env.new_string("clipboard")?;
        let service = env
            .call_method(
                activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&name)],
            )?
            .l()?;
        let label = env.new_string("Horizon")?;
        let body = env.new_string(text)?;
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[JValue::Object(&label), JValue::Object(&body)],
            )?
            .l()?;
        env.call_method(
            &service,
            "setPrimaryClip",
            "(Landroid/content/ClipData;)V",
            &[JValue::Object(&clip)],
        )?;
        Ok(())
    })
}

/// The text on the system clipboard, if any.
pub fn read_text() -> Result<String, String> {
    with_activity(|env, activity| {
        let name = env.new_string("clipboard")?;
        let service = env
            .call_method(
                activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(&name)],
            )?
            .l()?;
        let clip = env
            .call_method(
                &service,
                "getPrimaryClip",
                "()Landroid/content/ClipData;",
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(String::new());
        }
        let item = env
            .call_method(
                &clip,
                "getItemAt",
                "(I)Landroid/content/ClipData$Item;",
                &[JValue::Int(0)],
            )?
            .l()?;
        let text = env
            .call_method(&item, "getText", "()Ljava/lang/CharSequence;", &[])?
            .l()?;
        if text.is_null() {
            return Ok(String::new());
        }
        let string = env
            .call_method(&text, "toString", "()Ljava/lang/String;", &[])?
            .l()?;
        let jstr = jni::objects::JString::from(string);
        let out: String = env.get_string(&jstr)?.into();
        Ok(out)
    })
}

/// Starts the app's launch screen in a new task and ends this process, so the game starts
/// fresh with the files it just prepared (winit can't make a second event loop in a process).
pub fn restart() -> Result<(), String> {
    with_activity(|env, activity| {
        let manager = env
            .call_method(
                activity,
                "getPackageManager",
                "()Landroid/content/pm/PackageManager;",
                &[],
            )?
            .l()?;
        let package = env
            .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
            .l()?;
        let intent = env
            .call_method(
                &manager,
                "getLaunchIntentForPackage",
                "(Ljava/lang/String;)Landroid/content/Intent;",
                &[JValue::Object(&package)],
            )?
            .l()?;
        // FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK
        env.call_method(
            &intent,
            "addFlags",
            "(I)Landroid/content/Intent;",
            &[JValue::Int(0x1000_8000)],
        )?;
        env.call_method(
            activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    })?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    std::process::exit(0)
}
