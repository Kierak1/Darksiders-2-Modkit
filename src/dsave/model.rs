pub const MAGIC: &[u8; 4] = b"DSAV";
pub const FILE_VERSION: u32 = 0x8000_000D;
pub const MODULE_VERSION: u32 = 15;
pub const RAW_BLOCK: usize = 0x4000;

#[derive(Debug, Clone, PartialEq)]
pub struct Save {
    pub endian: u8,
    pub info: Info,
    pub strings: Vec<String>,
    pub module: Module,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub format: u8,
    pub endian: u8,
    pub b17: u8,
    pub b18: u8,
    pub b19: u8,
    pub zero: u8,
    pub f20: u32,
    pub f24: u32,
    pub f28: u32,
    pub f36: u32,
    pub f40: u32,
    pub f44: u32,
    pub f48: u32,
    pub counter: u32,
    pub global_version: u32,
    pub flags: u32,
    pub b56: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub version: u32,
    pub name: String,
    pub loc_records: Vec<[String; 5]>,
    pub f32: u32,
    pub f40: String,
    pub f48: u32,
    pub f52: u32,
    pub f56: u32,
    pub f60: u32,
    pub f64: u32,
    pub f68: u32,
    pub f72: u32,
    pub objects: Vec<Object>,
    pub v1_first: u32,
    pub v1: Vec<V1Record>,
    pub n3: Vec<(String, u8, u8, u8)>,
    pub n4: Vec<(u32, u8, u8)>,
    pub n5: Vec<String>,
    pub inventory: Vec<ItemList>,
    pub equip_marker: u32,
    pub equipped: Vec<EquipSlot>,
    pub records: Vec<Record>,
    pub state96: State96,
    pub raw: Vec<u8>,
    pub s80: Mod80,
    pub tail: Vec<Vec<u64>>,
    pub last: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub class: String,
    pub props: Vec<MapEntry>,
    pub elems: Vec<ObjElem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapEntry {
    pub hash: u64,
    pub name: String,
    pub flag: Option<u8>,
}

impl MapEntry {
    pub fn new(hash: u64, name: String, flag: Option<u8>) -> Self {
        Self { hash, name, flag }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjElem {
    pub kind: u16,
    pub children: Vec<ObjChild>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjChild {
    pub kind: u16,
    pub leaves: Vec<ObjLeaf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObjLeaf {
    pub data: u32,
    pub tag: u8,
    pub value: LeafValue,
    pub props: Vec<MapEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LeafValue {
    None,
    Two(u8, u8),
    Pair(Option<[u32; 3]>, Option<[u32; 3]>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct V1Record {
    pub name: String,
    pub inner: Vec<V1Inner>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct V1Inner {
    pub name: String,
    pub a: u16,
    pub b: u16,
    pub c: u32,
    pub d: u32,
    pub e: u32,
    pub f: u32,
    pub g: String,
    pub h: u32,
    pub i: u32,
    pub j: u32,
    pub k: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemList {
    pub name: String,
    pub field: u32,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    pub body: ItemBody,
}

impl Item {
    /// The `SaveItem` body of any save-item variant (`None` for base/ItemModifier
    /// items and nested `ItemModifier` objects).
    pub fn save_body(&self) -> Option<&SaveItem> {
        match &self.body {
            ItemBody::Save(s) | ItemBody::SaveU32(s, _) | ItemBody::SaveU32U8(s, _, _) => {
                Some(s.as_ref())
            }
            _ => None,
        }
    }

    pub fn save_body_mut(&mut self) -> Option<&mut SaveItem> {
        match &mut self.body {
            ItemBody::Save(s) | ItemBody::SaveU32(s, _) | ItemBody::SaveU32U8(s, _, _) => {
                Some(s.as_mut())
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemBody {
    Base([u8; 3], u32),
    Save(Box<SaveItem>),
    SaveU32(Box<SaveItem>, u32),
    SaveU32U8(Box<SaveItem>, u32, u8),
    Modifier(Box<ItemModifier>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveItem {
    pub base: ([u8; 3], u32),
    pub b816: u8,
    pub f872: u32,
    pub f820: u32,
    pub f844: u32,
    pub s856: String,
    pub s864: String,
    pub nested: Vec<Item>,
    pub desc: Option<Desc>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Desc {
    PossessedComponent {
        a: u32,
        b: u32,
        c: u32,
        entries: Vec<[u32; 3]>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemModifier {
    pub base: ([u8; 3], u32),
    pub attr: String,
    pub fields: [u32; 5],
    pub mods: Vec<Modifier>,
    pub effects: Vec<Gc48>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Modifier {
    pub name: String,
    pub a: [u32; 3],
    pub s: [String; 4],
    pub extra: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gc48 {
    pub b: u8,
    pub f: [u32; 4],
    pub obj: RefObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RefObject {
    pub name: String,
    pub body: RefBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RefBody {
    Script(ScriptBase),
    Combat(ScriptBase, [u32; 9], u8, Vec<Modifier>),
    Enhancement(ScriptBase, u8, String),
    Freeze(ScriptBase, u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptBase {
    pub flag: u8,
    pub refs: Vec<RefObject>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EquipSlot {
    pub f1: u32,
    pub f2: u32,
    pub b24: u8,
    pub f32: u64,
    pub b40: u8,
    pub b41: Option<u8>,
    pub s1: String,
    pub s2: String,
    pub s3: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub name: String,
    pub a: u32,
    pub b: u32,
    pub lists: Vec<ItemList>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State96 {
    pub name: String,
    pub a: Vec<Ef300>,
    pub b: Vec<Ef300>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ef300 {
    pub name: String,
    pub rows: Vec<(String, u8, u8, u32, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mod80 {
    pub version: u32,
    pub a: [u32; 27],
    pub v1: Option<u32>,
    pub f1: u32,
    pub f124: Option<u32>,
    pub f132: Option<u32>,
    pub b: [u32; 23],
    pub c1: Option<u32>,
    pub c2: Option<(u32, u32)>,
    pub d: [u32; 4],
    pub slots: Vec<Slot80>,
    pub u16_tail: Option<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slot80 {
    pub a: u32,
    pub name: String,
    pub rows: Vec<(u64, u32)>,
}
