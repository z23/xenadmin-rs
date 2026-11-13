// SPDX-License-Identifier: LGPL-2.1-only WITH OCaml-LGPL-linking-exception

//! The XAPI of the qualified toolstack, as Rust: a catalogue of its classes, fields,
//! messages and enumerations, a view of a record for every class, constants for every
//! enumeration value, and a type for every message that says what the call takes.
//!
//! All of it is generated when the crate is built, from the API description in
//! `data/xenapi.json`: what upstream's own generator writes for the data model of the
//! toolstack release named in `data/SOURCE`. See `NOTICE` for where it comes from and
//! `LICENSE` for the terms of this crate, which differ from the rest of the workspace.
//!
//! The views return values as the host sent them. What a value must look like, how large
//! it may be and what happens when it is malformed are for the caller to decide.

/// One class of the API.
pub struct Class {
    pub name: &'static str,
    pub fields: &'static [Field],
    pub messages: &'static [Message],
    /// The names of the enumerations its fields and messages use.
    pub enumerations: &'static [&'static str],
}
/// One field of a class, with its type as the description writes it.
pub struct Field {
    pub name: &'static str,
    pub kind: &'static str,
    /// `RW`, `RO/runtime` or `RO/constructor`.
    pub qualifier: &'static str,
    /// Where the field is in its lifecycle, such as `Published_s` or `Deprecated_s`.
    pub state: &'static str,
}
/// One message of a class: a call. `parameters` are the types in order, the session
/// included where the call takes one. The first `required` of them must be given; the
/// rest may be left out from the end, and the host then uses its defaults.
pub struct Message {
    pub name: &'static str,
    pub parameters: &'static [&'static str],
    pub required: usize,
    pub result: &'static str,
    pub state: &'static str,
}
/// One enumeration with its values in the API's order.
pub struct Enumeration {
    pub name: &'static str,
    pub values: &'static [&'static str],
}

/// A call: one message of the API with its arguments, each in the type of its kind and
/// under the name the API gives it. The types in [`calls`] are these; what a call is
/// given is checked where it is written, by the compiler: how many arguments, which is
/// which, and a flag, a number, text or a reference of the class the API names.
///
/// ```
/// use xenadmin_xapi::{Call, calls, refs};
/// let call = calls::vm::Start {
///     vm: refs::vm("OpaqueRef:vm"),
///     start_paused: false,
///     force: true,
/// };
/// assert_eq!(
///     call.task(),
///     ("Async.VM.start", vec!["OpaqueRef:vm".into(), false.into(), true.into()])
/// );
/// ```
///
/// A reference of another class, an argument of another kind or a missing argument does
/// not compile:
///
/// ```compile_fail,E0308
/// use xenadmin_xapi::{calls, refs};
/// let _ = calls::vm::Start {
///     vm: refs::host("OpaqueRef:host"),
///     start_paused: false,
///     force: true,
/// };
/// ```
///
/// ```compile_fail,E0308
/// use xenadmin_xapi::{calls, refs};
/// let _ = calls::vm::Start { vm: refs::vm("OpaqueRef:vm"), start_paused: "no", force: true };
/// ```
///
/// ```compile_fail,E0063
/// use xenadmin_xapi::{calls, refs};
/// let _ = calls::vm::Start { vm: refs::vm("OpaqueRef:vm"), force: true };
/// ```
pub trait Call: Sized {
    /// The call's name: `Class.message`.
    const NAME: &'static str;
    /// Its name when the host is to run it as a task: `Async.Class.message`.
    const ASYNC: &'static str;
    /// The type of what the call gives, as the description writes it.
    const RESULT: &'static str;
    /// The arguments after the session, in the API's order, as the JSON that is sent.
    fn arguments(self) -> Vec<serde_json::Value>;
    /// The call by its name, with its arguments.
    fn request(self) -> (&'static str, Vec<serde_json::Value>) {
        (Self::NAME, self.arguments())
    }
    /// The call as a task: the host answers with the task's reference.
    fn task(self) -> (&'static str, Vec<serde_json::Value>) {
        (Self::ASYNC, self.arguments())
    }
}

/// A reference to an object of class `C`, as an argument of a call. One is made with the
/// function for its class in [`refs`], from text or from a value a host sent. The class is
/// the caller's word, written where the reference is used; the compiler holds it against
/// the parameter the reference is given to.
pub struct Ref<'a, C> {
    source: RefSource<'a>,
    class: std::marker::PhantomData<fn() -> C>,
}
/// What a reference is made from: text, or a value as a host sent it (a field of a
/// record, the answer to a call), which goes back unread.
#[derive(Clone, Copy)]
pub struct RefSource<'a>(Sent<'a>);
#[derive(Clone, Copy)]
enum Sent<'a> {
    Text(&'a str),
    Json(&'a serde_json::Value),
}
impl<'a> From<&'a str> for RefSource<'a> {
    fn from(reference: &'a str) -> Self {
        Self(Sent::Text(reference))
    }
}
impl<'a> From<&'a String> for RefSource<'a> {
    fn from(reference: &'a String) -> Self {
        Self(Sent::Text(reference))
    }
}
impl<'a> From<&'a serde_json::Value> for RefSource<'a> {
    fn from(value: &'a serde_json::Value) -> Self {
        Self(Sent::Json(value))
    }
}
impl<'a> From<RefField<'a>> for RefSource<'a> {
    fn from(field: RefField<'a>) -> Self {
        Self(Sent::Json(field.0))
    }
}
impl<'a> From<FieldValue<'a>> for RefSource<'a> {
    fn from(field: FieldValue<'a>) -> Self {
        Self(Sent::Json(field.0))
    }
}
impl<'a, C> Ref<'a, C> {
    /// A reference the caller says is of class `C`.
    pub fn new(source: impl Into<RefSource<'a>>) -> Self {
        Self {
            source: source.into(),
            class: std::marker::PhantomData,
        }
    }
    /// The JSON that is sent for it.
    pub fn json(&self) -> serde_json::Value {
        match self.source.0 {
            Sent::Text(reference) => serde_json::Value::String(reference.to_owned()),
            Sent::Json(value) => value.clone(),
        }
    }
}
impl<C> Clone for Ref<'_, C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<C> Copy for Ref<'_, C> {}

/// A whole number as an argument of a call. Calls send it as a string of digits, which is
/// how the API's own bindings write a 64-bit integer.
#[derive(Clone, Copy)]
pub struct Int(i128);
impl Int {
    /// The JSON that is sent for it.
    pub fn json(self) -> serde_json::Value {
        serde_json::Value::String(self.0.to_string())
    }
}
macro_rules! int_from {
    ($($number:ty),+) => {
        $(impl From<$number> for Int {
            fn from(number: $number) -> Self {
                Self(number.into())
            }
        })+
    };
}
int_from!(i8, i16, i32, i64, u8, u16, u32, u64);
/// A number behind a reference converts as the number does.
impl<T: Copy + Into<Int>> From<&T> for Int {
    fn from(number: &T) -> Self {
        (*number).into()
    }
}
impl From<usize> for Int {
    fn from(number: usize) -> Self {
        // A usize is at most 64 bits on every platform Rust has.
        Self(number as i128)
    }
}

/// One field of a record, as the host sent it: null when the record does not carry it.
///
/// A field is not a record. Code that reads a field takes this type, so a whole record
/// cannot be handed to it by mistake. It dereferences to the JSON value for reading.
#[derive(Clone, Copy, Debug)]
pub struct FieldValue<'a>(&'a serde_json::Value);
impl<'a> FieldValue<'a> {
    /// A value that did not come through a record view: a key inside a map-valued field,
    /// or a read by key where the record's class is not known.
    pub fn of(value: &'a serde_json::Value) -> Self {
        Self(value)
    }
    /// The value with the record's lifetime, for a result that outlives this field.
    pub fn value(self) -> &'a serde_json::Value {
        self.0
    }
}
impl std::ops::Deref for FieldValue<'_> {
    type Target = serde_json::Value;
    fn deref(&self) -> &serde_json::Value {
        self.0
    }
}
impl<T> PartialEq<T> for FieldValue<'_>
where
    serde_json::Value: PartialEq<T>,
{
    fn eq(&self, other: &T) -> bool {
        self.0 == other
    }
}
/// A JSON value compares with a field as with the value the field wraps. With the
/// comparison above, this also lets two fields be compared.
impl PartialEq<FieldValue<'_>> for serde_json::Value {
    fn eq(&self, other: &FieldValue<'_>) -> bool {
        self == other.0
    }
}
/// A field is written out as the value it wraps, so it can be sent back in a call.
impl serde::Serialize for FieldValue<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// The types a view's methods return, one for each kind of field the API has.
///
/// The API gives every field a type. A view's method returns the field as the host sent
/// it, wrapped in the type for that kind, so that what is done with the field is checked
/// against its kind where it is written: a boolean field compares with a boolean and not
/// with a string, a reference is not accepted where a set is read, and a reader that
/// takes one kind is not handed another. A typed field does not read as the JSON value
/// it wraps: it has the readers of its kind and no others, so reading a whole number as
/// text, or a reference as a set, does not compile.
///
/// ```
/// let record = serde_json::json!({"VCPUs_max": 2, "resident_on": "OpaqueRef:host"});
/// let vm = xenadmin_xapi::records::VmRecord::new(&record);
/// assert_eq!(vm.vcpus_max().unsigned(), Some(2));
/// assert_eq!(vm.resident_on().reference(), Some("OpaqueRef:host"));
/// ```
///
/// The same record, read with the JSON accessor of another kind:
///
/// ```compile_fail,E0599
/// let record = serde_json::json!({"VCPUs_max": 2, "resident_on": "OpaqueRef:host"});
/// let vm = xenadmin_xapi::records::VmRecord::new(&record);
/// assert_eq!(vm.vcpus_max().as_str(), None);
/// ```
///
/// ```compile_fail,E0599
/// let record = serde_json::json!({"VCPUs_max": 2, "resident_on": "OpaqueRef:host"});
/// let vm = xenadmin_xapi::records::VmRecord::new(&record);
/// assert_eq!(vm.resident_on().as_array(), None);
/// ```
///
/// [`value`](TextField::value) gives the JSON value for the few reads no reader covers,
/// and a field turns into a [`FieldValue`] where the kind does not matter.
///
/// The readers here turn a well-formed value into its Rust type and give `None` for
/// anything else. Limits on sizes and what a malformed value means stay with the caller.
macro_rules! typed_field {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug)]
        pub struct $name<'a>(&'a serde_json::Value);
        impl<'a> $name<'a> {
            /// A value of this kind that did not come through a record view, such as a
            /// key inside a map-valued field.
            pub fn of(value: &'a serde_json::Value) -> Self {
                Self(value)
            }
            /// The JSON value as the host sent it, for the few reads no reader here covers.
            /// What is read from it is no longer checked against the field's kind by the
            /// compiler, so prefer the kind's own reader.
            pub fn value(self) -> &'a serde_json::Value {
                self.0
            }
            /// Whether the record does not carry the field, or carries null for it.
            pub fn is_null(self) -> bool {
                self.0.is_null()
            }
        }
        impl<'a> From<$name<'a>> for FieldValue<'a> {
            fn from(field: $name<'a>) -> Self {
                FieldValue(field.0)
            }
        }
        /// Two fields of one kind compare as their values do.
        impl PartialEq for $name<'_> {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }
        impl PartialEq<serde_json::Value> for $name<'_> {
            fn eq(&self, other: &serde_json::Value) -> bool {
                self.0 == other
            }
        }
        impl PartialEq<$name<'_>> for serde_json::Value {
            fn eq(&self, other: &$name<'_>) -> bool {
                self == other.0
            }
        }
        impl serde::Serialize for $name<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(serializer)
            }
        }
    };
}
/// The Rust types a field of one kind compares with.
macro_rules! compares_with {
    ($name:ident: $($other:ty),+) => {
        $(impl PartialEq<$other> for $name<'_> {
            fn eq(&self, other: &$other) -> bool {
                *self.0 == *other
            }
        })+
    };
}
typed_field!(
    /// A `bool` field.
    FlagField
);
typed_field!(
    /// A `string` field.
    TextField
);
typed_field!(
    /// A `datetime` field, which hosts send as text.
    TimeField
);
typed_field!(
    /// A reference to an object: a field of a type `X ref`.
    RefField
);
typed_field!(
    /// A field whose type is an enumeration; its values are in [`enumerations`].
    ChoiceField
);
typed_field!(
    /// An `int` field. Hosts send whole numbers as numbers or as strings of digits.
    WholeField
);
typed_field!(
    /// A `float` field.
    RealField
);
typed_field!(
    /// A field that is a set of values.
    SetField
);
typed_field!(
    /// A field that is a map from keys to values.
    MapField
);
/// The kinds a reader can ask for; see [`Reads`].
pub mod kind {
    /// A flag: a `bool` field.
    pub struct Flag;
    /// Text: a `string` field, a `datetime` or the name of an enumeration's value.
    pub struct Text;
    /// A reference to an object.
    pub struct Reference;
    /// A whole number.
    pub struct Whole;
    /// A real number.
    pub struct Real;
    /// A set of values.
    pub struct Set;
    /// A map from keys to values.
    pub struct Map;
}
/// What a reader of one kind of value accepts: a field of that kind, or a JSON value that
/// did not come through a view, whose kind the caller vouches for by passing it. A
/// function that reads references takes `impl Reads<'a, kind::Reference>`: a reference
/// field is accepted, a number field or a set is refused by the compiler, and a value
/// read by key or taken out of a map goes in as before.
///
/// ```
/// use xenadmin_xapi::{Reads, kind, records::VmRecord};
/// fn reference<'a>(value: impl Reads<'a, kind::Reference>) -> Option<&'a str> {
///     value.raw().as_str()
/// }
/// let record = serde_json::json!({"VCPUs_max": 2, "resident_on": "OpaqueRef:host"});
/// let vm = VmRecord::new(&record);
/// assert_eq!(reference(vm.resident_on()), Some("OpaqueRef:host"));
/// assert_eq!(reference(&record["resident_on"]), Some("OpaqueRef:host"));
/// ```
///
/// ```compile_fail,E0277
/// use xenadmin_xapi::{Reads, kind, records::VmRecord};
/// fn reference<'a>(value: impl Reads<'a, kind::Reference>) -> Option<&'a str> {
///     value.raw().as_str()
/// }
/// let record = serde_json::json!({"VCPUs_max": 2, "resident_on": "OpaqueRef:host"});
/// let vm = VmRecord::new(&record);
/// assert_eq!(reference(vm.vcpus_max()), None);
/// ```
pub trait Reads<'a, K> {
    /// The JSON value to read.
    fn raw(self) -> &'a serde_json::Value;
}
impl<'a, K> Reads<'a, K> for &'a serde_json::Value {
    fn raw(self) -> &'a serde_json::Value {
        self
    }
}
/// A plain field, whose kind is not known, is accepted like a JSON value.
impl<'a, K> Reads<'a, K> for FieldValue<'a> {
    fn raw(self) -> &'a serde_json::Value {
        self.0
    }
}
macro_rules! reads {
    ($($field:ident as $kind:ident),+) => {
        $(impl<'a> Reads<'a, kind::$kind> for $field<'a> {
            fn raw(self) -> &'a serde_json::Value {
                self.0
            }
        })+
    };
}
reads!(
    FlagField as Flag,
    TextField as Text,
    TimeField as Text,
    ChoiceField as Text,
    RefField as Reference,
    WholeField as Whole,
    RealField as Real,
    SetField as Set,
    MapField as Map
);
/// A field that hosts send as text and that is read as text: a string, a time or the
/// name of an enumeration's value. A reference is text as well, but it is not read as
/// what it says, so it is not one of these.
pub trait Textual<'a>: Copy + Into<FieldValue<'a>> {
    /// The text, when the host sent a string.
    fn text(self) -> Option<&'a str>;
}
impl<'a> Textual<'a> for TextField<'a> {
    fn text(self) -> Option<&'a str> {
        self.0.as_str()
    }
}
impl<'a> Textual<'a> for TimeField<'a> {
    fn text(self) -> Option<&'a str> {
        self.0.as_str()
    }
}
impl<'a> Textual<'a> for ChoiceField<'a> {
    fn text(self) -> Option<&'a str> {
        self.0.as_str()
    }
}
compares_with!(FlagField: bool);
compares_with!(TextField: str, &str, String);
compares_with!(TimeField: str, &str, String);
compares_with!(RefField: str, &str, String);
compares_with!(ChoiceField: str, &str, String);
compares_with!(WholeField: i32, i64, u32, u64, usize, str, &str, String);
compares_with!(RealField: f64, i64, u64);
impl FlagField<'_> {
    /// The boolean, when the host sent one.
    pub fn get(self) -> Option<bool> {
        self.0.as_bool()
    }
}
impl<'a> TextField<'a> {
    /// The text, when the host sent a string.
    pub fn text(self) -> Option<&'a str> {
        self.0.as_str()
    }
}
impl<'a> TimeField<'a> {
    /// The time as the host wrote it, when it sent a string.
    pub fn text(self) -> Option<&'a str> {
        self.0.as_str()
    }
}
impl<'a> RefField<'a> {
    /// The reference, when the host sent a string. The null reference is a reference.
    pub fn reference(self) -> Option<&'a str> {
        self.0.as_str()
    }
}
impl<'a> ChoiceField<'a> {
    /// The value's name, when the host sent a string.
    pub fn name(self) -> Option<&'a str> {
        self.0.as_str()
    }
}
impl WholeField<'_> {
    /// The number, sent as a number or as a string of digits.
    pub fn number(self) -> Option<i64> {
        self.0
            .as_i64()
            .or_else(|| self.0.as_str().and_then(|digits| digits.parse().ok()))
    }
    /// The number when it is not negative, sent as a number or as a string of digits.
    pub fn unsigned(self) -> Option<u64> {
        self.0
            .as_u64()
            .or_else(|| self.0.as_str().and_then(|digits| digits.parse().ok()))
    }
}
impl RealField<'_> {
    /// The number, sent as a number or as its text.
    pub fn number(self) -> Option<f64> {
        self.0
            .as_f64()
            .or_else(|| self.0.as_str().and_then(|text| text.parse().ok()))
    }
}
impl<'a> SetField<'a> {
    /// The members, when the host sent a list.
    pub fn members(self) -> Option<&'a Vec<serde_json::Value>> {
        self.0.as_array()
    }
    /// The members as text, when the host sent a list of strings and nothing else. Every
    /// set the API has is of references, strings or enumeration values but one.
    pub fn strings(self) -> Option<Vec<&'a str>> {
        self.0
            .as_array()?
            .iter()
            .map(serde_json::Value::as_str)
            .collect()
    }
}
impl<'a> MapField<'a> {
    /// The entries, when the host sent a map.
    pub fn entries(self) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
        self.0.as_object()
    }
    /// The value under `key`, when the host sent a map that has the key.
    pub fn get(self, key: &str) -> Option<&'a serde_json::Value> {
        self.0.as_object()?.get(key)
    }
    /// The text under `key`, when the host sent a map whose value there is a string.
    /// Nearly every map the API has is from strings to strings, references or
    /// enumeration values.
    pub fn text(self, key: &str) -> Option<&'a str> {
        self.get(key)?.as_str()
    }
}
/// The value under a key of a map-valued field, null when the map lacks it. A value
/// inside a map has no kind of its own: what is read from it is not checked.
impl std::ops::Index<&str> for MapField<'_> {
    type Output = serde_json::Value;
    fn index(&self, key: &str) -> &serde_json::Value {
        &self.0[key]
    }
}

/// A check, in debug builds, that the records given to views are of the views' classes.
///
/// A view cannot know that its record is of its class: a record does not say what it is,
/// and a field of the same name in another class reads just as well. A record does carry
/// every field of its class, though, so a record of another class has keys the view's
/// class lacks, and so has a test fixture that carries a field the API does not have.
///
/// When the environment variable `XENADMIN_XAPI_VIEW_AUDIT` names a file, every view made
/// in a debug build compares the keys of its record with the fields of its class and
/// appends a line for each key the class lacks: the class, the key and where the view was
/// made, separated by tabs, once per process. Only names are written, never a value.
/// Without the variable nothing is compared, and release builds contain none of this.
///
/// The same file receives what is wrong with the arguments of the calls a debug build
/// sends, when the caller hands them to [`audit::observe_call`]: a line that starts with
/// `call`, then the call's name, the position of the parameter in the API's order (the
/// session is the first) and the type the API gives it with the kind of value it was
/// given. A call given more arguments than the API takes, or fewer than it requires, is
/// reported with `count` for its position; parameters the API marks as optional may be
/// left out from the end. Here too only names and kinds are written, never a value.
/// Every call observed is also listed once per process, as `sent` and its name, and every
/// place that made a view as `viewed`, its class and the place, so that what the two
/// checks covered can be counted afterwards.
#[cfg(debug_assertions)]
pub mod audit {
    use std::collections::BTreeSet;
    use std::io::Write as _;
    use std::sync::{Mutex, OnceLock};

    struct Sink {
        file: std::fs::File,
        seen: BTreeSet<(&'static str, String, &'static str, u32)>,
        calls: BTreeSet<String>,
    }
    static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();

    fn sink() -> Option<&'static Mutex<Sink>> {
        SINK.get_or_init(|| {
            let path = std::env::var_os("XENADMIN_XAPI_VIEW_AUDIT")?;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()?;
            Some(Mutex::new(Sink {
                file,
                seen: BTreeSet::new(),
                calls: BTreeSet::new(),
            }))
        })
        .as_ref()
    }

    fn given(value: &serde_json::Value) -> &'static str {
        match value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "a boolean",
            serde_json::Value::Number(_) => "a number",
            serde_json::Value::String(_) => "a string",
            serde_json::Value::Array(_) => "a list",
            serde_json::Value::Object(_) => "a map",
        }
    }

    /// What `value` is, when it is not what the API's type `kind` asks for: the kind of
    /// JSON value, or for a list or a map the first member that does not fit. A type this
    /// does not know fits anything.
    ///
    /// Whole numbers may be sent as numbers or as strings of digits, which is how the
    /// toolstack reads them. A reference is any string: a record's reference and the
    /// null reference look alike to it.
    pub fn misfit(kind: &str, value: &serde_json::Value) -> Option<String> {
        use serde_json::Value;
        if let Some(member) = kind.strip_suffix(" set") {
            let Value::Array(members) = value else {
                return Some(given(value).into());
            };
            return members
                .iter()
                .find_map(|v| misfit(member, v))
                .map(|what| format!("a list holding {what}"));
        }
        if let Some((key, member)) = kind
            .strip_prefix('(')
            .and_then(|k| k.strip_suffix(") map"))
            .and_then(|k| k.split_once(" -> "))
        {
            let Value::Object(members) = value else {
                return Some(given(value).into());
            };
            return members.iter().find_map(|(k, v)| {
                if let Some(what) = misfit(key, &Value::String(k.clone())) {
                    return Some(format!("a map with a key that is {what}"));
                }
                misfit(member, v).map(|what| format!("a map holding {what}"))
            });
        }
        if let Some(name) = kind.strip_prefix("enum ") {
            let Value::String(text) = value else {
                return Some(given(value).into());
            };
            let known = super::ENUMERATIONS
                .binary_search_by(|e| e.name.cmp(name))
                .is_ok_and(|at| super::ENUMERATIONS[at].values.contains(&text.as_str()));
            return (!known).then(|| "a string that is none of its values".into());
        }
        let fits = match kind {
            "string" | "datetime" => value.is_string(),
            "bool" => value.is_boolean(),
            "int" => {
                value.is_i64()
                    || value.is_u64()
                    || value.as_str().is_some_and(|v| v.parse::<i64>().is_ok())
            }
            "float" => {
                value.is_number() || value.as_str().is_some_and(|v| v.parse::<f64>().is_ok())
            }
            _ if kind.ends_with(" ref") => value.is_string(),
            _ if kind.ends_with(" record") => value.is_object(),
            _ => true,
        };
        if fits {
            None
        } else if kind == "int" && value.is_string() {
            Some("a string that is not a whole number".into())
        } else {
            Some(given(value).into())
        }
    }

    /// What is wrong with the arguments of one call, the session's token included where
    /// the call takes a session: for each, the position and what [`misfit`] says, as the
    /// lines' last two fields. Nothing for a call the catalogue does not have.
    pub fn call_misfits(method: &str, arguments: &[serde_json::Value]) -> Vec<String> {
        let name = method.strip_prefix("Async.").unwrap_or(method);
        let Some(message) = name.split_once('.').and_then(|(class, message)| {
            let class = &super::CLASSES[super::CLASSES
                .binary_search_by(|c| c.name.cmp(class))
                .ok()?];
            let at = class
                .messages
                .binary_search_by(|m| m.name.cmp(message))
                .ok()?;
            Some(&class.messages[at])
        }) else {
            return Vec::new();
        };
        if !(message.required..=message.parameters.len()).contains(&arguments.len()) {
            let takes = if message.required == message.parameters.len() {
                message.required.to_string()
            } else {
                format!("{} to {}", message.required, message.parameters.len())
            };
            return vec![format!("count\ttakes {takes}, given {}", arguments.len())];
        }
        message
            .parameters
            .iter()
            .zip(arguments)
            .enumerate()
            .filter_map(|(at, (kind, value))| {
                misfit(kind, value).map(|what| format!("{}\t{kind} given {what}", at + 1))
            })
            .collect()
    }

    /// Called by whatever sends a call, with the arguments as they go on the wire.
    pub fn observe_call(method: &str, arguments: &[serde_json::Value]) {
        let Some(sink) = sink() else {
            return;
        };
        let misfits = call_misfits(method, arguments);
        let Ok(mut sink) = sink.lock() else {
            return;
        };
        let lines = std::iter::once(format!("sent\t{method}\n")).chain(
            misfits
                .into_iter()
                .map(|misfit| format!("call\t{method}\t{misfit}\n")),
        );
        for line in lines {
            if sink.calls.insert(line.clone()) {
                let _ = sink.file.write_all(line.as_bytes());
            }
        }
    }

    /// The keys of `record` that are not fields of `class`; none for a value that is not
    /// an object or a class the catalogue does not have.
    pub fn foreign_keys<'a>(class: &str, record: &'a serde_json::Value) -> Vec<&'a str> {
        let (Some(object), Ok(index)) = (
            record.as_object(),
            super::CLASSES.binary_search_by(|c| c.name.cmp(class)),
        ) else {
            return Vec::new();
        };
        let fields = super::CLASSES[index].fields;
        object
            .keys()
            .map(String::as_str)
            .filter(|key| fields.binary_search_by(|f| f.name.cmp(key)).is_err())
            .collect()
    }

    /// Called by every view when it is made, with the place that made it.
    pub fn observe(
        class: &'static str,
        record: &serde_json::Value,
        place: &'static std::panic::Location<'static>,
    ) {
        let Some(sink) = sink() else {
            return;
        };
        // Each place is listed once; a thread asks the shared record only about a place
        // it has not met itself, so that views made in a loop cost a lookup and no lock.
        thread_local! {
            static MET: std::cell::RefCell<BTreeSet<(usize, u32, u32)>> =
                const { std::cell::RefCell::new(BTreeSet::new()) };
        }
        let key = (place.file().as_ptr() as usize, place.line(), place.column());
        if MET.with(|met| met.borrow_mut().insert(key))
            && let Ok(mut sink) = sink.lock()
        {
            let line = format!("viewed\t{class}\t{}:{}\n", place.file(), place.line());
            if sink.calls.insert(line.clone()) {
                let _ = sink.file.write_all(line.as_bytes());
            }
        }
        let foreign = foreign_keys(class, record);
        if foreign.is_empty() {
            return;
        }
        let Ok(mut sink) = sink.lock() else {
            return;
        };
        for key in foreign {
            if sink
                .seen
                .insert((class, key.to_owned(), place.file(), place.line()))
            {
                // One short line at a time, so that processes appending to one file do not
                // interleave within a line.
                let line = format!("{class}\t{key}\t{}:{}\n", place.file(), place.line());
                let _ = sink.file.write_all(line.as_bytes());
            }
        }
    }
}

/// The upstream tag, commit and digest of the description the bindings were generated from.
pub const SOURCE: &str = include_str!("../data/SOURCE");

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(test)]
#[path = "../generator.rs"]
mod generator;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn the_catalogue_is_the_whole_description_in_order() {
        assert_eq!(CLASSES.len(), 70);
        assert_eq!(CLASSES.iter().map(|c| c.fields.len()).sum::<usize>(), 899);
        assert_eq!(
            CLASSES.iter().map(|c| c.messages.len()).sum::<usize>(),
            1943
        );
        assert!(CLASSES.windows(2).all(|pair| pair[0].name < pair[1].name));
        assert!(
            ENUMERATIONS
                .windows(2)
                .all(|pair| pair[0].name < pair[1].name)
        );
        assert!(SOURCE.contains("tag=v26.1.11"));
        let vm = CLASSES.iter().find(|c| c.name == "VM").unwrap();
        let start = vm.messages.iter().find(|m| m.name == "start").unwrap();
        assert_eq!(start.parameters, ["session ref", "VM ref", "bool", "bool"]);
        assert_eq!((start.required, start.result), (4, "void"));
        // A snapshot may leave out the disks to ignore, which the description marks.
        let snapshot = vm.messages.iter().find(|m| m.name == "snapshot").unwrap();
        assert_eq!((snapshot.required, snapshot.parameters.len()), (3, 4));
        assert_eq!(
            CLASSES
                .iter()
                .flat_map(|c| c.messages)
                .map(|m| m.parameters.len() - m.required)
                .sum::<usize>(),
            77
        );
        assert!(vm.enumerations.contains(&"vm_power_state"));
        let field = vm.fields.iter().find(|f| f.name == "VCPUs_max").unwrap();
        assert_eq!((field.kind, field.qualifier), ("int", "RO/constructor"));
    }

    #[test]
    fn a_view_reads_its_class_fields_and_a_missing_field_is_null() {
        let record =
            json!({"uuid": "u", "power_state": "Halted", "VCPUs_max": 2, "is_a_snapshot": false});
        let vm = records::VmRecord::new(&record);
        assert_eq!(records::VmRecord::CLASS, "VM");
        assert_eq!(vm.uuid(), "u");
        assert_eq!(vm.power_state(), enumerations::vm_power_state::HALTED);
        assert_eq!(vm.vcpus_max(), 2);
        assert_eq!(vm.is_a_snapshot(), false);
        assert!(vm.snapshot_of().is_null());
        assert!(records::HostRecord::new(&Value::Null).enabled().is_null());
        assert_eq!(vm.record(), &record);
        // A reader gives a value with the record's lifetime, not the field's.
        let name: &str = vm.uuid().text().unwrap();
        assert_eq!(name, "u");
        assert_eq!(FieldValue::of(&record["VCPUs_max"]), 2);
        assert_eq!(vm.vcpus_max().unsigned(), Some(2));
        // The JSON value itself is there to ask for, by name.
        assert_eq!(vm.vcpus_max().value(), &json!(2));
        // Two fields of one kind compare as their values do. Fields of different kinds,
        // such as a UUID and a power state, do not compare at all: that does not compile.
        assert!(vm.uuid() == records::VmRecord::new(&record).uuid());
        assert!(vm.uuid() != vm.name_label());
        // A JSON value compares with a field, and a field serializes as its value.
        assert!(json!("u") == vm.uuid());
        assert_eq!(json!([vm.uuid(), vm.vcpus_max()]), json!(["u", 2]));
        // A field whose name is a Rust keyword keeps its meaning with an underscore.
        assert_eq!(
            records::VdiRecord::new(&json!({"type": "user"})).type_(),
            "user"
        );
    }

    #[test]
    fn a_field_has_the_type_of_its_kind_and_reads_as_it() {
        let record = json!({
            "is_a_snapshot": true, "name_label": "guest", "power_state": "Running",
            "resident_on": "OpaqueRef:host", "VCPUs_max": "4", "memory_overhead": 1048576,
            "VBDs": ["OpaqueRef:a", "OpaqueRef:b"], "other_config": {"k": "v"},
            "snapshot_time": "20261112T00:00:00Z",
        });
        let vm = records::VmRecord::new(&record);
        // Each kind has its own type, named here so that a change of kind does not compile.
        let flag: FlagField = vm.is_a_snapshot();
        let text: TextField = vm.name_label();
        let choice: ChoiceField = vm.power_state();
        let reference: RefField = vm.resident_on();
        let whole: WholeField = vm.vcpus_max();
        let set: SetField = vm.vbds();
        let map: MapField = vm.other_config();
        let time: TimeField = vm.snapshot_time();
        // A field compares with the Rust type of its kind.
        assert!(flag == true && text == "guest" && choice == enumerations::vm_power_state::RUNNING);
        let owned = String::from("OpaqueRef:host");
        assert!(reference == "OpaqueRef:host" && reference == owned);
        // A whole number is read whether the host sent digits or a number.
        assert_eq!(
            (whole.number(), vm.memory_overhead().number()),
            (Some(4), Some(1048576))
        );
        assert!(whole == "4" && vm.memory_overhead() == 1048576_u64);
        // The readers give the Rust value, or nothing when the host sent something else.
        assert_eq!(flag.get(), Some(true));
        assert_eq!(
            (text.text(), choice.name(), time.text()),
            (Some("guest"), Some("Running"), Some("20261112T00:00:00Z"))
        );
        assert_eq!(reference.reference(), Some("OpaqueRef:host"));
        assert_eq!(set.members().map(Vec::len), Some(2));
        assert_eq!(set.strings(), Some(vec!["OpaqueRef:a", "OpaqueRef:b"]));
        assert_eq!(SetField::of(&json!(["a", 1])).strings(), None);
        assert_eq!(map.entries().and_then(|m| m.get("k")), Some(&json!("v")));
        // One key of a map: its value, its text, or null when the map lacks it.
        assert_eq!(
            (map.get("k"), map.text("k"), map.text("absent")),
            (Some(&json!("v")), Some("v"), None)
        );
        assert_eq!((&map["k"], &map["absent"]), (&json!("v"), &Value::Null));
        assert_eq!(vm.platform().get("k"), None);
        assert_eq!(
            (whole.unsigned(), WholeField::of(&json!(-1)).unsigned()),
            (Some(4), None)
        );
        assert_eq!(vm.is_control_domain().get(), None);
        assert_eq!(vm.tags().members(), None);
        assert_eq!(FlagField::of(&json!("true")).get(), None);
        assert_eq!(WholeField::of(&json!("four")).number(), None);
        assert_eq!(RealField::of(&json!("1.5")).number(), Some(1.5));
        // Text, a time and an enumeration's value are read as text by one reader; a
        // reference is not text to read, and a set or a flag is no text at all.
        fn read<'a>(field: impl Textual<'a>) -> Option<&'a str> {
            field.text()
        }
        assert_eq!(
            (read(text), read(choice), read(time)),
            (Some("guest"), Some("Running"), Some("20261112T00:00:00Z"))
        );
        // Any of them is a plain field where the kind does not matter, and still reads
        // as the JSON value it wraps.
        let plain: FieldValue = set.into();
        assert_eq!(plain.as_array().map(Vec::len), Some(2));
        assert_eq!(
            json!({"f": flag, "t": text}),
            json!({"f": true, "t": "guest"})
        );
        // A field of a kind there is no type for stays a plain field.
        let _: FieldValue = records::EventRecord::new(&record).snapshot();
    }

    #[test]
    fn a_reader_takes_fields_of_its_kind_and_values_that_came_by_key() {
        fn number<'a>(value: impl Reads<'a, kind::Whole>) -> Option<u64> {
            WholeField::of(value.raw()).unsigned()
        }
        fn text<'a>(value: impl Reads<'a, kind::Text>) -> Option<&'a str> {
            value.raw().as_str()
        }
        fn count<'a>(value: impl Reads<'a, kind::Set>) -> Option<usize> {
            value.raw().as_array().map(Vec::len)
        }
        let record = json!({
            "name_label": "guest", "power_state": "Running", "VCPUs_max": "4",
            "VBDs": ["OpaqueRef:a"], "snapshot_time": "20261112T00:00:00Z",
        });
        let vm = records::VmRecord::new(&record);
        // A field of the reader's kind, a value read by key and a plain field all go in.
        assert_eq!(number(vm.vcpus_max()), Some(4));
        assert_eq!(number(&record["VCPUs_max"]), Some(4));
        assert_eq!(number(FieldValue::of(&record["VCPUs_max"])), Some(4));
        assert_eq!(count(vm.vbds()), Some(1));
        // Text, a time and an enumeration's value are all text to a reader of text.
        assert_eq!(
            (
                text(vm.name_label()),
                text(vm.snapshot_time()),
                text(vm.power_state())
            ),
            (Some("guest"), Some("20261112T00:00:00Z"), Some("Running"))
        );
        // `number(vm.vbds())` and `count(vm.vcpus_max())` do not compile; the examples on
        // `Reads` and on the field types hold that. A field of another kind goes in only
        // as its JSON value, asked for by name.
        assert_eq!(number(vm.name_label().value()), None);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_audit_names_the_keys_a_class_lacks_and_nothing_else() {
        // A VM's own fields are not reported, whatever their values.
        let vm =
            json!({"uuid": "u", "power_state": "Halted", "VBDs": [], "other_config": {"k": "v"}});
        assert!(audit::foreign_keys("VM", &vm).is_empty());
        // A repository's record read as a VM is told apart by the keys only it has, and a
        // field the API does not have at all is reported the same way.
        let sr = json!({"uuid": "u", "allowed_operations": [], "PBDs": [], "content_type": "user"});
        assert_eq!(audit::foreign_keys("VM", &sr), ["PBDs", "content_type"]);
        assert!(audit::foreign_keys("SR", &sr).is_empty());
        assert_eq!(
            audit::foreign_keys("VBD", &json!({"uuid": "u", "reserved": false})),
            ["reserved"]
        );
        // Values that are not records, and classes the catalogue lacks, report nothing.
        assert!(audit::foreign_keys("VM", &Value::Null).is_empty());
        assert!(audit::foreign_keys("VM", &json!("OpaqueRef:NULL")).is_empty());
        assert!(audit::foreign_keys("no such class", &sr).is_empty());
        // Making a view without the variable set compares nothing and changes nothing.
        assert_eq!(records::VmRecord::new(&sr).uuid(), "u");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_audit_judges_a_call_by_the_types_the_api_gives_its_parameters() {
        use audit::{call_misfits, misfit};
        // VM.start takes the session, the VM, and two booleans.
        let good = [
            json!("OpaqueRef:s"),
            json!("OpaqueRef:vm"),
            json!(false),
            json!(true),
        ];
        assert!(call_misfits("VM.start", &good).is_empty());
        assert!(call_misfits("Async.VM.start", &good).is_empty());
        // Two arguments in each other's places are both named, by position and type.
        let swapped = [
            json!("OpaqueRef:s"),
            json!(false),
            json!("OpaqueRef:vm"),
            json!(true),
        ];
        assert_eq!(
            call_misfits("VM.start", &swapped),
            ["2\tVM ref given a boolean", "3\tbool given a string"]
        );
        // One too few is a matter of count, and nothing more is said about it.
        assert_eq!(
            call_misfits("VM.start", &good[..3]),
            ["count\ttakes 4, given 3"]
        );
        // A parameter the API marks as optional may be left out, and is judged when given.
        let snapshot = [json!("OpaqueRef:s"), json!("OpaqueRef:vm"), json!("name")];
        assert!(call_misfits("VM.snapshot", &snapshot).is_empty());
        assert_eq!(
            call_misfits("VM.snapshot", &snapshot[..2]),
            ["count\ttakes 3 to 4, given 2"]
        );
        let mut with_disks = snapshot.to_vec();
        with_disks.push(json!("OpaqueRef:vdi"));
        assert_eq!(
            call_misfits("VM.snapshot", &with_disks),
            ["4\tVDI ref set given a string"]
        );
        with_disks.push(json!(true));
        assert_eq!(
            call_misfits("VM.snapshot", &with_disks),
            ["count\ttakes 3 to 4, given 5"]
        );
        // A call the API does not have is for the conformance test, not for this one.
        assert!(call_misfits("VM.no_such_call", &good).is_empty());
        assert!(call_misfits("nothing", &[]).is_empty());
        // Whole numbers go as numbers or as digits; anything else is named.
        assert_eq!(misfit("int", &json!(4)), None);
        assert_eq!(misfit("int", &json!("2147483648")), None);
        assert_eq!(
            misfit("int", &json!("4 GiB")).as_deref(),
            Some("a string that is not a whole number")
        );
        assert_eq!(misfit("int", &json!(1.5)).as_deref(), Some("a number"));
        assert_eq!(misfit("float", &json!(1.5)), None);
        // An enumeration takes its own values and no others, in their own case.
        assert_eq!(misfit("enum vm_power_state", &json!("Halted")), None);
        assert_eq!(
            misfit("enum vm_power_state", &json!("halted")).as_deref(),
            Some("a string that is none of its values")
        );
        // Lists and maps are judged by their members, and say which kind did not fit.
        assert_eq!(misfit("string set", &json!(["a", "b"])), None);
        assert_eq!(
            misfit("string set", &json!(["a", 1])).as_deref(),
            Some("a list holding a number")
        );
        assert_eq!(
            misfit("VM ref set", &json!("OpaqueRef:vm")).as_deref(),
            Some("a string")
        );
        assert_eq!(misfit("(string -> string) map", &json!({"k": "v"})), None);
        assert_eq!(
            misfit("(string -> string) map", &json!({"k": true})).as_deref(),
            Some("a map holding a boolean")
        );
        assert_eq!(
            misfit(
                "(enum vm_power_state -> string) map",
                &json!({"Stopped": "v"})
            )
            .as_deref(),
            Some("a map with a key that is a string that is none of its values")
        );
        assert_eq!(misfit("VM record", &json!({})), None);
        assert_eq!(misfit("VM record", &json!([])).as_deref(), Some("a list"));
        assert_eq!(misfit("datetime", &Value::Null).as_deref(), Some("null"));
        // A type this does not know is not judged.
        assert_eq!(misfit("some new type", &json!(1)), None);
    }

    #[test]
    fn enumeration_constants_are_the_api_values() {
        use enumerations::vm_power_state;
        assert_eq!(
            vm_power_state::ALL,
            ["Halted", "Paused", "Running", "Suspended"]
        );
        assert_eq!(vm_power_state::RUNNING, "Running");
        let listed = ENUMERATIONS
            .iter()
            .find(|e| e.name == "vm_power_state")
            .unwrap();
        assert_eq!(listed.values, vm_power_state::ALL);
        assert_eq!(enumerations::task_status_type::PENDING, "pending");
    }

    #[test]
    fn a_call_names_its_arguments_and_sends_them_as_the_api_takes_them() {
        use calls::{host, pif, vdi, vm};
        // One parameter is a tuple, none a unit, several are named.
        assert_eq!(
            vm::GetRecord(refs::vm("OpaqueRef:a")).request(),
            ("VM.get_record", vec![json!("OpaqueRef:a")])
        );
        assert_eq!(
            vm::GetAllRecords.request(),
            ("VM.get_all_records", Vec::new())
        );
        let owned = String::from("OpaqueRef:d");
        // A whole number goes as digits, from any integer type, and the task's name is
        // the call's with `Async.` before it.
        assert_eq!(
            vdi::Resize {
                vdi: refs::vdi(&owned),
                size: 1_073_741_824_u64.into(),
            }
            .task(),
            (
                "Async.VDI.resize",
                vec![json!("OpaqueRef:d"), json!("1073741824")]
            )
        );
        assert_eq!(Int::from(-1_i32).json(), json!("-1"));
        // Text and an enumeration's value go as strings, in the API's order whatever the
        // order they are written in.
        assert_eq!(
            pif::ReconfigureIp {
                dns: "192.0.2.53",
                gateway: "192.0.2.1",
                netmask: "255.255.255.0",
                ip: "192.0.2.7",
                mode: enumerations::ip_configuration_mode::STATIC,
                self_: refs::pif("OpaqueRef:p"),
            }
            .arguments(),
            [
                json!("OpaqueRef:p"),
                json!("Static"),
                json!("192.0.2.7"),
                json!("255.255.255.0"),
                json!("192.0.2.1"),
                json!("192.0.2.53")
            ]
        );
        // Parameters that may be left out: the plain type takes the required ones.
        assert_eq!(host::Evacuate(refs::host("h")).arguments(), [json!("h")]);
        assert_eq!(
            host::EvacuateWithNetwork {
                host: refs::host("h"),
                network: refs::network("n"),
            }
            .arguments(),
            [json!("h"), json!("n")]
        );
        assert_eq!(
            host::EvacuateWithEvacuateBatchSize {
                host: refs::host("h"),
                network: refs::network("n"),
                evacuate_batch_size: 4_u8.into(),
            }
            .arguments(),
            [json!("h"), json!("n"), json!("4")]
        );
        // A reference a host sent goes back as it came, whatever it holds.
        let record = json!({"resident_on": "OpaqueRef:h", "affinity": null});
        let view = records::VmRecord::new(&record);
        assert_eq!(
            host::GetRecord(refs::host(view.resident_on())).arguments(),
            [json!("OpaqueRef:h")]
        );
        assert_eq!(
            host::GetRecord(refs::host(view.affinity())).arguments(),
            [Value::Null]
        );
        assert_eq!(
            host::GetRecord(refs::host(&record["resident_on"])).arguments(),
            [json!("OpaqueRef:h")]
        );
        // A set, a map and a record are JSON values, handed over whole.
        assert_eq!(
            vm::SetOtherConfig {
                self_: refs::vm("v"),
                value: json!({"k": "v"}),
            }
            .request(),
            ("VM.set_other_config", vec![json!("v"), json!({"k": "v"})])
        );
        assert_eq!(<vm::Start as Call>::RESULT, "void");
        assert_eq!(<vm::GetByUuid as Call>::RESULT, "VM ref");
        // Every message that takes the session has a type, and one more for each
        // parameter it lets a caller leave out: the catalogue's count against the
        // generated source's.
        let source = include_str!(concat!(env!("OUT_DIR"), "/bindings.rs"));
        let with_session = || {
            CLASSES
                .iter()
                .flat_map(|class| class.messages)
                .filter(|message| message.parameters.first() == Some(&"session ref"))
        };
        let types: usize = with_session()
            .map(|message| 1 + message.parameters.len() - message.required)
            .sum();
        assert_eq!((with_session().count(), types), (1941, 2016));
        assert_eq!(source.matches("const NAME: &'static str = ").count(), types);
    }

    #[test]
    fn the_generator_sorts_names_keeps_types_in_order_and_refuses_clashes() {
        let description = br#"[
          {"name":"b_thing","fields":[],"messages":[],"enums":[]},
          {"name":"A","enums":[{"name":"mode","values":[{"name":"on"},{"name":"On"},{"name":"9p"}]}],
           "fields":[{"name":"type","type":"string","qualifier":"RW","lifecycle":{"state":"Published_s"}},
                     {"name":"Max_size","type":"int","qualifier":"RO/runtime","lifecycle":{"state":"Deprecated_s"}}],
           "messages":[{"name":"start","params":[{"type":"session ref"},{"type":"A ref"},
                                                 {"type":"bool","required":false}],
                        "result":["void"],"lifecycle":{"state":"Published_s"}},
                       {"name":"get","params":[],"result":["string","the value"],"lifecycle":{"state":"Published_s"}}]}
        ]"#;
        let out = generator::generate(description).unwrap();
        assert!(out.find("name: \"A\"").unwrap() < out.find("name: \"b_thing\"").unwrap());
        assert!(out.contains(
            "Message { name: \"start\", parameters: &[\"session ref\", \"A ref\", \"bool\"], required: 2, result: \"void\", state: \"Published_s\" },"
        ));
        assert!(out.contains("Message { name: \"get\", parameters: &[], required: 0, result: \"string\", state: \"Published_s\" },"));
        assert!(out.contains("pub struct ARecord<'a>(&'a Value);"));
        assert!(out.contains("pub struct BThingRecord<'a>(&'a Value);"));
        assert!(out.contains("pub fn type_(self) -> TextField<'a> {"));
        assert!(out.contains("pub fn max_size(self) -> WholeField<'a> {"));
        assert!(out.contains("pub const ON: &str = \"on\";"));
        assert!(out.contains("pub const ON_: &str = \"On\";"));
        assert!(out.contains("pub const _9P: &str = \"9p\";"));
        assert!(out.contains("pub const ALL: &[&str] = &[ON, ON_, _9P];"));
        // A message that takes the session has a type: a tuple with one parameter, named
        // fields with more, and one more type for each parameter that may be left out.
        // A parameter the description does not name is named for its place.
        assert!(out.contains("pub struct A;") && out.contains("pub struct BThing;"));
        assert!(out.contains(
            "pub fn b_thing<'a>(reference: impl Into<RefSource<'a>>) -> Ref<'a, class::BThing> {"
        ));
        assert!(out.contains("pub struct Start<'a>(\n"));
        assert!(out.contains("            pub Ref<'a, class::A>,\n"));
        assert!(out.contains("pub struct StartWithArgument1<'a> {\n"));
        assert!(out.contains("            pub argument_1: bool,\n"));
        assert!(out.contains("const NAME: &'static str = \"A.start\";"));
        assert!(out.contains("const ASYNC: &'static str = \"Async.A.start\";"));
        assert!(out.contains("vec![self.argument_0.json(), Value::Bool(self.argument_1)]"));
        // One that does not take the session has none.
        assert!(!out.contains("pub struct Get"));
        assert_eq!(out, generator::generate(description).unwrap());

        let same = br#"[{"name":"A","enums":[],"fields":[],
          "messages":[{"name":"go","params":[{"type":"session ref","name":"session_id"},
                       {"type":"A ref","name":"VM"},{"type":"A ref","name":"vm"}],
                       "result":["void"],"lifecycle":{"state":"Published_s"}}]}]"#;
        let error = generator::generate(same).unwrap_err();
        assert!(
            error.contains("parameters of A.go: VM and vm both become vm"),
            "{error}"
        );

        let clash = br#"[{"name":"A","enums":[],"messages":[],
          "fields":[{"name":"MTU","type":"int","qualifier":"RW","lifecycle":{"state":"Published_s"}},
                    {"name":"mtu","type":"int","qualifier":"RW","lifecycle":{"state":"Published_s"}}]}]"#;
        let error = generator::generate(clash).unwrap_err();
        assert!(error.contains("MTU and mtu both become mtu"), "{error}");

        // Arguments can only be left out from the end, so a required parameter after an
        // optional one could not be described by a count and is refused.
        let gap = br#"[{"name":"A","enums":[],"fields":[],
          "messages":[{"name":"go","params":[{"type":"bool","required":false},{"type":"A ref"}],
                       "result":["void"],"lifecycle":{"state":"Published_s"}}]}]"#;
        let error = generator::generate(gap).unwrap_err();
        assert!(
            error.contains("A.go has a required parameter after an optional one"),
            "{error}"
        );
    }
}
