use super::items::set_item_field;
use super::model::{
    Desc, Gc48, Info, Item, ItemBody, ItemList, ItemModifier, Mod80, Modifier, Module, RefBody,
    RefObject, Save, SaveItem, ScriptBase, State96,
};
use super::parse::{parse_item, Reader};
use super::write::{path_with_bak, write_item, Writer};
use std::path::Path;

fn sample_modifier(name: &str, extra: Option<u32>) -> Modifier {
    Modifier {
        name: name.to_string(),
        a: [1, 42, u32::MAX],
        s: [
            "Defense".to_string(),
            "base/Mod_Armor_Boots".to_string(),
            "Defense".to_string(),
            String::new(),
        ],
        extra,
    }
}

fn sample_save_item() -> Item {
    let modifier = Item {
        name: "ItemModifier".to_string(),
        body: ItemBody::Modifier(Box::new(ItemModifier {
            base: ([0x11, 1, 0], u32::MAX),
            attr: "$Item.Attributes.Defense".to_string(),
            fields: [3, 0, 0, 0, 0],
            mods: vec![sample_modifier("PossessedAttributeModifier", Some(0))],
            effects: vec![Gc48 {
                b: 0,
                f: [100, 0, 1, 0],
                obj: RefObject {
                    name: "EnhancementEffect".to_string(),
                    body: RefBody::Enhancement(
                        ScriptBase {
                            flag: 0,
                            refs: Vec::new(),
                        },
                        1,
                        "items/enhancement_test".to_string(),
                    ),
                },
            }],
        })),
    };
    Item {
        name: "armor/armor_feet_test".to_string(),
        body: ItemBody::Save(Box::new(SaveItem {
            base: ([1, 1, 0], u32::MAX),
            b816: 0,
            f872: u32::MAX,
            f820: u32::MAX,
            f844: u32::MAX,
            s856: String::new(),
            s864: "base/mod_armor_boots".to_string(),
            nested: vec![modifier],
            desc: None,
        })),
    }
}

#[test]
fn synthetic_item_roundtrip() {
    let item = sample_save_item();
    let mut w = Writer::new();
    write_item(&mut w, &item);
    let item_len = w.out.len();
    w.bytes(&[2, 0, b'x', b'x']);
    let bytes = w.out;
    let mut r = Reader::new(&bytes);
    let parsed = parse_item(&mut r).unwrap();
    assert_eq!(r.c.pos, item_len);
    assert_eq!(parsed, item);
}

#[test]
fn synthetic_variant_bodies_roundtrip() {
    for body in [
        ItemBody::SaveU32(Box::new(RoundTripBody::save()), 10),
        ItemBody::SaveU32U8(Box::new(RoundTripBody::save()), 5, 1),
    ] {
        let item = Item {
            name: "items/item_test".to_string(),
            body,
        };
        let mut w = Writer::new();
        write_item(&mut w, &item);
        let item_len = w.out.len();
        w.bytes(&[2, 0, b'x', b'x']);
        let bytes = w.out;
        let mut r = Reader::new(&bytes);
        let parsed = parse_item(&mut r).unwrap();
        assert_eq!(r.c.pos, item_len);
        assert_eq!(parsed, item);
    }
}

struct RoundTripBody;
impl RoundTripBody {
    fn save() -> SaveItem {
        SaveItem {
            base: ([1, 1, 0], u32::MAX),
            b816: 0,
            f872: 4,
            f820: u32::MAX,
            f844: u32::MAX,
            s856: String::new(),
            s864: String::new(),
            nested: Vec::new(),
            desc: Some(Desc::PossessedComponent {
                a: 0,
                b: 17,
                c: 19,
                entries: vec![[1, 2, 3]],
            }),
        }
    }
}

#[test]
fn field_edit_paths() {
    let mut item = sample_save_item();
    let change = set_item_field(&mut item, "mod[0].mods[0].a[1]", "77").unwrap();
    assert!(change.contains("77"));
    let change = set_item_field(&mut item, "mod[0].mods[0].extra", "3").unwrap();
    assert!(change.contains('3'));
    let change = set_item_field(&mut item, "s864", "base/other").unwrap();
    assert!(change.contains("base/other"));
    let change = set_item_field(&mut item, "mod[0].effects[0].obj.s", "items/x").unwrap();
    assert!(change.contains("items/x"));
    assert!(set_item_field(&mut item, "mod[9].mods[0].a[0]", "1").is_err());
    assert!(set_item_field(&mut item, "mod[0].mods[0].a[7]", "1").is_err());
}

#[test]
fn backup_path_is_sibling() {
    assert_eq!(
        path_with_bak(Path::new("C:\\saves\\dssave0.dsav")),
        std::path::PathBuf::from("C:\\saves\\dssave0.dsav.bak")
    );
}

// ---------------------------------------------------------------------------
// synthetic builders shared by the other unit-test modules (cfg(test) only)

/// A save item with one `ItemModifier` per `(attribute, enum_id, a[2])` stat.
pub(crate) fn synthetic_item(name: &str, stats: &[(&str, u32, u32)], b816: u8) -> Item {
    let nested: Vec<Item> = stats
        .iter()
        .map(|(attr, enum_id, value)| Item {
            name: "ItemModifier".to_string(),
            body: ItemBody::Modifier(Box::new(ItemModifier {
                base: ([0, 1, 0], u32::MAX),
                attr: format!("$Item.Attributes.{attr}"),
                fields: [1, 0, 0, 0, 0],
                mods: vec![Modifier {
                    name: "AttributeModifier".to_string(),
                    a: [1, *enum_id, *value],
                    s: [
                        attr.to_string(),
                        String::new(),
                        String::new(),
                        String::new(),
                    ],
                    extra: None,
                }],
                effects: Vec::new(),
            })),
        })
        .collect();
    Item {
        name: name.to_string(),
        body: ItemBody::Save(Box::new(SaveItem {
            base: ([1, 1, 0], u32::MAX),
            b816,
            f872: u32::MAX,
            f820: u32::MAX,
            f844: u32::MAX,
            s856: String::new(),
            s864: String::new(),
            nested,
            desc: None,
        })),
    }
}

/// A minimal valid save with the given inventory lists.
pub(crate) fn synthetic_save(inventory: Vec<(&str, Vec<Item>)>) -> Save {
    Save {
        endian: 0,
        info: Info {
            format: 6,
            endian: 0,
            b17: 0,
            b18: 0,
            b19: 0,
            zero: 0,
            f20: 0,
            f24: 0,
            f28: 0,
            f36: 0,
            f40: 0,
            f44: 0,
            f48: 0,
            counter: 0,
            global_version: 0,
            flags: 0,
            b56: 0,
        },
        strings: Vec::new(),
        module: Module {
            version: 15,
            name: String::new(),
            loc_records: Vec::new(),
            f32: 0,
            f40: String::new(),
            f48: 0,
            f52: 0,
            f56: 0,
            f60: 0,
            f64: 0,
            f68: 0,
            f72: 0,
            objects: Vec::new(),
            v1_first: 0,
            v1: Vec::new(),
            n3: Vec::new(),
            n4: Vec::new(),
            n5: Vec::new(),
            inventory: inventory
                .into_iter()
                .map(|(name, items)| ItemList {
                    name: name.to_string(),
                    field: 1000,
                    items,
                })
                .collect(),
            equip_marker: 1,
            equipped: Vec::new(),
            records: Vec::new(),
            state96: State96 {
                name: String::new(),
                a: Vec::new(),
                b: Vec::new(),
            },
            raw: vec![0; super::model::RAW_BLOCK],
            s80: Mod80 {
                version: 4,
                a: [0; 27],
                v1: None,
                f1: 0,
                f124: None,
                f132: None,
                b: [0; 23],
                c1: None,
                c2: None,
                d: [0; 4],
                slots: Vec::new(),
                u16_tail: None,
            },
            tail: Vec::new(),
            last: 0,
        },
    }
}
