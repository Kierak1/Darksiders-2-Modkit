pub mod bulk;
pub mod db;
mod items;
pub mod loc;
mod model;
mod parse;
pub mod possessed;
pub mod preset;
pub mod slots;
pub mod stats;
mod totals;
mod write;

pub use bulk::{apply_bulk, parse_bulk_op, BulkOp, BulkReport, BulkScope};
pub use db::{
    classify_raw, display_name, localized_name, pretty_name, AttrInfo, AttrKind, AttributeCatalog,
    Confidence, Coverage, DefAttr, DefModifier, DefValue, ItemDb, ItemDef,
};
pub use items::{
    add_item, container_fit, copy_item, equip_item, find_items, item_lines, modifier_summary,
    move_item, set_item_field, unequip_item, ItemRef, ItemSource,
};
pub use loc::LocTable;
pub use model::{
    Desc, Ef300, EquipSlot, Gc48, Info, Item, ItemBody, ItemList, ItemModifier, LeafValue,
    MapEntry, Mod80, Modifier, Module, ObjChild, ObjElem, ObjLeaf, Object, Record, RefBody,
    RefObject, Save, SaveItem, ScriptBase, Slot80, State96, V1Inner, V1Record, FILE_VERSION, MAGIC,
    MODULE_VERSION, RAW_BLOCK,
};
pub use parse::parse;
pub use possessed::{
    find_mod, find_stat, possessed_add_effect, possessed_add_mod, possessed_info, possessed_max,
    possessed_remove_effect, possessed_remove_mod, possessed_set_level, PossessedInfo,
};
pub use preset::{apply_preset, load_preset, parse_preset, Preset};
pub use slots::{default_save_dir, format_time, list_slots, SlotSummary};
pub use stats::{
    row_display, set_stat, set_stat_row, set_stat_row_offset, stat_views, StatView, ROW_RANGE,
    VALUE_MAX, VALUE_NONE,
};
pub use totals::{
    equipped_totals, format_value, sorted_items, total_line, totals, Total, TotalScope,
};
pub use write::{write_file, WriteReport};

#[cfg(test)]
mod tests;
