/*!
JNI bridge for Android integration with `com.velocenetwork.mobile.VeloceNative`.
*/

use std::sync::atomic::Ordering;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::{MobileConfig, MobileEngine};

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_startNode(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
    join_code: JString,
    mesh_port: jint,
) -> jboolean {
    let data_dir_str: String = match env.get_string(&data_dir) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    let join_code_opt: Option<String> = if !join_code.is_null() {
        env.get_string(&join_code).ok().map(|s| s.into())
    } else {
        None
    };

    let port = if mesh_port > 0 && mesh_port <= 65535 {
        mesh_port as u16
    } else {
        10550
    };

    let mut config = MobileConfig::default();
    config.data_dir = data_dir_str;
    config.mesh_port = port;
    config.join_code = join_code_opt;

    match MobileEngine::start(config) {
        Ok(_) => JNI_TRUE,
        Err(_) => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_stopNode(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    match MobileEngine::stop_global() {
        Ok(_) => JNI_TRUE,
        Err(_) => JNI_FALSE,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_isRunning(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if let Some(engine) = MobileEngine::get_global() {
        if engine.is_running.load(Ordering::SeqCst) {
            return JNI_TRUE;
        }
    }
    JNI_FALSE
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_getNodeStatus(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let json = if let Some(engine) = MobileEngine::get_global() {
        let status = engine.get_status();
        serde_json::to_string(&status).unwrap_or_else(|_| "{}".into())
    } else {
        r#"{"is_running":false,"peer_count":0}"#.to_string()
    };

    match env.new_string(json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_getPeers(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let json = if let Some(engine) = MobileEngine::get_global() {
        let peers = engine.get_peers();
        serde_json::to_string(&peers).unwrap_or_else(|_| "[]".into())
    } else {
        "[]".to_string()
    };

    match env.new_string(json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_getMeshKv(
    mut env: JNIEnv,
    _class: JClass,
    key: JString,
) -> jstring {
    let key_str: String = match env.get_string(&key) {
        Ok(s) => s.into(),
        Err(_) => return std::ptr::null_mut(),
    };

    if let Some(engine) = MobileEngine::get_global() {
        if let Some(val) = engine.get_kv(&key_str) {
            if let Ok(js) = env.new_string(val) {
                return js.into_raw();
            }
        }
    }
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_putMeshKv(
    mut env: JNIEnv,
    _class: JClass,
    key: JString,
    val: JString,
) -> jboolean {
    let key_str: String = match env.get_string(&key) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };
    let val_str: String = match env.get_string(&val) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    if let Some(engine) = MobileEngine::get_global() {
        if engine.put_kv(&key_str, &val_str).is_ok() {
            return JNI_TRUE;
        }
    }
    JNI_FALSE
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_resolveHostname(
    mut env: JNIEnv,
    _class: JClass,
    hostname: JString,
) -> jstring {
    let host_str: String = match env.get_string(&hostname) {
        Ok(s) => s.into(),
        Err(_) => return std::ptr::null_mut(),
    };

    if let Some(engine) = MobileEngine::get_global() {
        if let Some(ip) = engine.resolve_vln_hostname(&host_str) {
            if let Ok(js) = env.new_string(ip) {
                return js.into_raw();
            }
        }
    }
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "system" fn Java_com_velocenetwork_mobile_VeloceNative_getMetrics(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let text = if let Some(engine) = MobileEngine::get_global() {
        let status = engine.get_status();
        format!(
            "# HELP veloce_mobile_peers Number of active peers\n# TYPE veloce_mobile_peers gauge\nveloce_mobile_peers {}\n# HELP veloce_mobile_uptime Uptime in seconds\n# TYPE veloce_mobile_uptime counter\nveloce_mobile_uptime {}\n",
            status.peer_count, status.uptime_secs
        )
    } else {
        "# VeloceCore Mobile offline\n".to_string()
    };

    match env.new_string(text) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}
