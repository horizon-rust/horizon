//! JNI bridge stub - minimal types for compilation.

use std::marker::PhantomData;
use jni::objects::JObject;

/// Errors from JNI bridge operations.
#[derive(Debug, thiserror::Error)]
pub enum JniError {
    #[error("JNI error: {0}")]
    Jni(String),
    #[error("Null pointer: {0}")]
    NullPointer(String),
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
    #[error("Class not found: {0}")]
    ClassNotFound(String),
    #[error("Method not found: {class}.{method}{sig}")]
    MethodNotFound { class: String, method: String, sig: String },
    #[error("Field not found: {class}.{field}{sig}")]
    FieldNotFound { class: String, field: String, sig: String },
    #[error("Exception thrown: {0}")]
    Exception(String),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

/// Result type for JNI bridge operations.
pub type JniResult<T> = Result<T, JniError>;

/// Stub JNI environment.
pub struct JniEnv<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JniEnv<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
    
    pub fn class_loader(&self) -> &crate::class_loader::FabricClassLoader {
        unimplemented!()
    }
    
    pub fn check_exception(&mut self) -> JniResult<()> {
        Ok(())
    }
    
    pub fn find_class(&mut self, _name: &str) -> JniResult<JavaClass<'a>> {
        Ok(JavaClass::new())
    }
    
    pub fn find_class_fabric(&mut self, _name: &str) -> JniResult<JavaClass<'a>> {
        Ok(JavaClass::new())
    }
    
    pub fn new_global_ref<T: JavaObject<'a>>(&mut self, _obj: T) -> JniResult<GlobalRef<T>> {
        Ok(GlobalRef::new())
    }
    
    pub fn new_local_ref<T: JavaObject<'a>>(&mut self, _obj: T) -> JniResult<LocalRef<'a, T>> {
        Ok(LocalRef::new())
    }
    
    pub fn new_string(&mut self, _s: &str) -> JniResult<JavaString<'a>> {
        Ok(JavaString::new())
    }
    
    pub fn get_string(&mut self, _jstring: JObject) -> JniResult<String> {
        Ok(String::new())
    }
    
    pub fn call_static_method(
        &mut self,
        _class: &JavaClass,
        _name: &str,
        _sig: &str,
        _args: &[jni::objects::JValueGen<JObject>],
    ) -> JniResult<jni::objects::JValueGen<JObject>> {
        Ok(jni::objects::JValueGen::Void)
    }
    
    pub fn call_method(
        &mut self,
        _obj: &impl JavaObject<'a>,
        _name: &str,
        _sig: &str,
        _args: &[jni::objects::JValueGen<JObject>],
    ) -> JniResult<jni::objects::JValueGen<JObject>> {
        Ok(jni::objects::JValueGen::Void)
    }
    
    pub fn get_static_field(&mut self, _class: &JavaClass, _name: &str, _sig: &str) -> JniResult<jni::objects::JValueGen<JObject>> {
        Ok(jni::objects::JValueGen::Void)
    }
    
    pub fn get_field(&mut self, _obj: &impl JavaObject<'a>, _name: &str, _sig: &str) -> JniResult<jni::objects::JValueGen<JObject>> {
        Ok(jni::objects::JValueGen::Void)
    }
    
    pub fn set_static_field(&mut self, _class: &JavaClass, _name: &str, _sig: &str, _value: jni::objects::JValueGen<JObject>) -> JniResult<()> {
        Ok(())
    }
    
    pub fn set_field(&mut self, _obj: &impl JavaObject<'a>, _name: &str, _sig: &str, _value: jni::objects::JValueGen<JObject>) -> JniResult<()> {
        Ok(())
    }
    
    pub fn new_object(&mut self, _class: &JavaClass, _sig: &str, _args: &[jni::objects::JValueGen<JObject>]) -> JniResult<LocalRef<'a, JObject>> {
        Ok(LocalRef::new())
    }
    
    pub fn new_object_array(&mut self, _len: i32, _class: &JavaClass, _init: &impl JavaObject<'a>) -> JniResult<JavaObjectArray<'a>> {
        Ok(JavaObjectArray::new())
    }
    
    pub fn new_byte_array(&mut self, _len: i32) -> JniResult<JavaByteArray<'a>> {
        Ok(JavaByteArray::new())
    }
    
    pub fn new_short_array(&mut self, _len: i32) -> JniResult<JavaShortArray<'a>> {
        Ok(JavaShortArray::new())
    }
    
    pub fn new_int_array(&mut self, _len: i32) -> JniResult<JavaIntArray<'a>> {
        Ok(JavaIntArray::new())
    }
    
    pub fn new_long_array(&mut self, _len: i32) -> JniResult<JavaLongArray<'a>> {
        Ok(JavaLongArray::new())
    }
    
    pub fn new_float_array(&mut self, _len: i32) -> JniResult<JavaFloatArray<'a>> {
        Ok(JavaFloatArray::new())
    }
    
    pub fn new_double_array(&mut self, _len: i32) -> JniResult<JavaDoubleArray<'a>> {
        Ok(JavaDoubleArray::new())
    }
    
    pub fn new_boolean_array(&mut self, _len: i32) -> JniResult<JavaBooleanArray<'a>> {
        Ok(JavaBooleanArray::new())
    }
    
    pub fn byte_array_to_vec(&mut self, _array: &JavaByteArray<'a>) -> JniResult<Vec<u8>> {
        Ok(Vec::new())
    }
    
    pub fn vec_to_byte_array(&mut self, _data: &[u8]) -> JniResult<JavaByteArray<'a>> {
        Ok(JavaByteArray::new())
    }
}

/// Trait for types that can be used as Java objects.
pub trait JavaObject<'a> {
    fn as_obj(&self) -> jni::objects::JObject;
}

/// A Java class reference.
pub struct JavaClass<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaClass<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
    
    pub fn as_class(&self) -> jni::objects::JClass<'a> {
        unimplemented!()
    }
    
    pub fn name(&self) -> Option<String> {
        None
    }
}

impl<'a> JavaObject<'a> for JavaClass<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java String reference.
pub struct JavaString<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaString<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaString<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java object array.
pub struct JavaObjectArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaObjectArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
    
    pub fn length(&self, _env: &mut JniEnv) -> JniResult<i32> {
        Ok(0)
    }
    
    pub fn get(&self, _env: &mut JniEnv, _index: i32) -> JniResult<LocalRef<'a, jni::objects::JObject>> {
        Ok(LocalRef::new())
    }
    
    pub fn set(&self, _env: &mut JniEnv, _index: i32, _value: &impl JavaObject<'a>) -> JniResult<()> {
        Ok(())
    }
}

impl<'a> JavaObject<'a> for JavaObjectArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java byte array.
pub struct JavaByteArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaByteArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaByteArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java short array.
pub struct JavaShortArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaShortArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaShortArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java int array.
pub struct JavaIntArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaIntArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaIntArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java long array.
pub struct JavaLongArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaLongArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaLongArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java float array.
pub struct JavaFloatArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaFloatArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaFloatArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java double array.
pub struct JavaDoubleArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaDoubleArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaDoubleArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A Java boolean array.
pub struct JavaBooleanArray<'a> {
    _marker: PhantomData<&'a ()>,
}

impl<'a> JavaBooleanArray<'a> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
}

impl<'a> JavaObject<'a> for JavaBooleanArray<'a> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}

/// A global reference to a Java object (persists across JNI calls).
pub struct GlobalRef<T> {
    _marker: PhantomData<T>,
}

impl<T> GlobalRef<T> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
    
    pub fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
    
    pub fn is_null(&self) -> bool {
        false
    }
    
    pub fn as_raw(&self) -> &jni::objects::GlobalRef {
        unimplemented!()
    }
}

/// A local reference to a Java object (valid only in current JNI frame).
pub struct LocalRef<'a, T> {
    _marker: PhantomData<(&'a (), T)>,
}

impl<'a, T> LocalRef<'a, T> {
    pub fn new() -> Self {
        Self { _marker: PhantomData }
    }
    
    pub fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
    
    pub fn is_null(&self) -> bool {
        false
    }
}

impl<'a, T> JavaObject<'a> for LocalRef<'a, T> {
    fn as_obj(&self) -> jni::objects::JObject {
        unimplemented!()
    }
}