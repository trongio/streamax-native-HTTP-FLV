/*
 * JNI bridge for streamax-core. Built as libstreamax_jni.so.
 * Links against libstreamax_core.so (the Rust cdylib).
 *
 * Builds via CMake; see CMakeLists.txt in this directory.
 */
#include <jni.h>
#include <string.h>

#include "streamax_core.h"

#define KIND_VIDEO_CONFIG 1
#define KIND_VIDEO_FRAME  2
#define KIND_AUDIO_CONFIG 3
#define KIND_AUDIO_FRAME  4
#define KIND_ERROR        99

#define JNI_FUNC(rt, name) \
    JNIEXPORT rt JNICALL Java_com_example_streamax_StreamaxCore_##name

JNI_FUNC(jlong, nativeNew)(JNIEnv* env, jobject self) {
    (void)env; (void)self;
    return (jlong)(intptr_t)streamax_demuxer_new();
}

JNI_FUNC(void, nativeFree)(JNIEnv* env, jobject self, jlong handle) {
    (void)env; (void)self;
    if (handle) streamax_demuxer_free((StreamaxDemuxer*)(intptr_t)handle);
}

JNI_FUNC(void, nativeReset)(JNIEnv* env, jobject self, jlong handle) {
    (void)env; (void)self;
    if (handle) streamax_demuxer_reset((StreamaxDemuxer*)(intptr_t)handle);
}

JNI_FUNC(void, nativeAppend)(JNIEnv* env, jobject self, jlong handle,
                              jbyteArray data, jint length) {
    (void)self;
    if (!handle || !data || length <= 0) return;
    jbyte* bytes = (*env)->GetByteArrayElements(env, data, NULL);
    if (!bytes) return;
    streamax_demuxer_append((StreamaxDemuxer*)(intptr_t)handle,
                            (const uint8_t*)bytes, (size_t)length);
    (*env)->ReleaseByteArrayElements(env, data, bytes, JNI_ABORT);
}

JNI_FUNC(jobject, nativeNextEvent)(JNIEnv* env, jobject self, jlong handle) {
    (void)self;
    if (!handle) return NULL;

    StreamaxEvent ev;
    if (!streamax_demuxer_next_event((StreamaxDemuxer*)(intptr_t)handle, &ev)) {
        return NULL;
    }

    jclass cls = (*env)->FindClass(env, "com/example/streamax/StreamaxCore$NativeEvent");
    if (!cls) return NULL;
    jmethodID ctor = (*env)->GetMethodID(env, cls, "<init>", "()V");
    if (!ctor) return NULL;
    jobject obj = (*env)->NewObject(env, cls, ctor);
    if (!obj) return NULL;

    /* Copy data bytes into a fresh jbyteArray so the Rust pointer's invalidation
     * (on the next FFI call) doesn't dangle. */
    jbyteArray jdata = (*env)->NewByteArray(env, (jsize)ev.data_len);
    if (jdata && ev.data && ev.data_len > 0) {
        (*env)->SetByteArrayRegion(env, jdata, 0, (jsize)ev.data_len, (const jbyte*)ev.data);
    }

    (*env)->SetIntField(env,     obj, (*env)->GetFieldID(env, cls, "kind",        "I"), (jint)ev.kind);
    (*env)->SetObjectField(env,  obj, (*env)->GetFieldID(env, cls, "data",        "[B"), jdata);
    (*env)->SetIntField(env,     obj, (*env)->GetFieldID(env, cls, "ptsMs",       "I"), (jint)ev.pts_ms);
    (*env)->SetBooleanField(env, obj, (*env)->GetFieldID(env, cls, "isKeyframe",  "Z"), ev.is_keyframe ? JNI_TRUE : JNI_FALSE);
    (*env)->SetIntField(env,     obj, (*env)->GetFieldID(env, cls, "width",       "I"), (jint)ev.width);
    (*env)->SetIntField(env,     obj, (*env)->GetFieldID(env, cls, "height",      "I"), (jint)ev.height);
    (*env)->SetIntField(env,     obj, (*env)->GetFieldID(env, cls, "sampleRate",  "I"), (jint)ev.sample_rate);
    (*env)->SetIntField(env,     obj, (*env)->GetFieldID(env, cls, "channels",    "I"), (jint)ev.channels);
    (*env)->SetIntField(env,     obj, (*env)->GetFieldID(env, cls, "objectType",  "I"), (jint)ev.object_type);
    return obj;
}
