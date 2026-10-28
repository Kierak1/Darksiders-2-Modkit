use std::path::Path;

pub(crate) fn run(cmd: &str, args: &[String]) -> Option<ds2_modkit::Result<()>> {
    match cmd {
        "head-remap" => Some(head_remap(args)),
        "armor-body" => Some(armor_body(args)),
        "armor-hide" => Some(armor_hide(args)),
        "head-write" => Some(head_write(args)),
        "head-scale" => Some(head_scale(args)),
        "head-hide" => Some(head_hide(args)),
        _ => None,
    }
}

fn head_remap(args: &[String]) -> ds2_modkit::Result<()> {
    let dec = args
        .get(1)
        .ok_or("usage: ds2-modkit head-remap <death.dec> <death.dec.stock>")?;
    let stock = args
        .get(2)
        .ok_or("usage: ds2-modkit head-remap <death.dec> <death.dec.stock>")?;
    let report = ds2_modkit::characters::mesh_head::remap_heads(Path::new(dec), Path::new(stock))?;
    for line in &report {
        println!("{line}");
    }
    Ok(())
}

fn armor_body(args: &[String]) -> ds2_modkit::Result<()> {
    let obj = args.get(1).ok_or(
        "usage: ds2-modkit armor-body <cj.obj> <armor.dec> <armor.dec.stock> [extra_arm_deg]",
    )?;
    let dec = args.get(2).ok_or(
        "usage: ds2-modkit armor-body <cj.obj> <armor.dec> <armor.dec.stock> [extra_arm_deg]",
    )?;
    let stock = args.get(3).ok_or(
        "usage: ds2-modkit armor-body <cj.obj> <armor.dec> <armor.dec.stock> [extra_arm_deg]",
    )?;
    let extra: f32 = args
        .get(4)
        .map(|s| s.parse().map_err(|_| "extra_arm_deg must be a number"))
        .transpose()?
        .unwrap_or(20.0);
    let report = ds2_modkit::characters::mesh_body::write_body(
        Path::new(obj),
        Path::new(dec),
        Path::new(stock),
        extra,
    )?;
    for line in &report {
        println!("{line}");
    }
    Ok(())
}

fn armor_hide(args: &[String]) -> ds2_modkit::Result<()> {
    let obpk_path = args
        .get(1)
        .ok_or("usage: ds2-modkit armor-hide <armor.obpk> <armor.dec> <part>...")?;
    let dec = args
        .get(2)
        .ok_or("usage: ds2-modkit armor-hide <armor.obpk> <armor.dec> <part>...")?;
    let names: Vec<String> = args[3..].to_vec();
    if names.is_empty() {
        return Err("usage: ds2-modkit armor-hide <armor.obpk> <armor.dec> <part>...".into());
    }
    let report = ds2_modkit::characters::armor_hide::hide_pieces(
        Path::new(obpk_path),
        Path::new(dec),
        &names,
    )?;
    for line in &report {
        println!("{line}");
    }
    Ok(())
}

fn head_write(args: &[String]) -> ds2_modkit::Result<()> {
    let obj = args
        .get(1)
        .ok_or("usage: ds2-modkit head-write <cj.obj> <death.dec>")?;
    let dec = args
        .get(2)
        .ok_or("usage: ds2-modkit head-write <cj.obj> <death.dec>")?;
    let report = ds2_modkit::characters::mesh_head::write_mask(Path::new(obj), Path::new(dec))?;
    for line in &report {
        println!("{line}");
    }
    Ok(())
}

fn head_scale(args: &[String]) -> ds2_modkit::Result<()> {
    let dec = args
        .get(1)
        .ok_or("usage: ds2-modkit head-scale <death.dec> <factor> [dz]")?;
    let factor: f32 = args
        .get(2)
        .ok_or("usage: ds2-modkit head-scale <death.dec> <factor> [dz]")?
        .parse()
        .map_err(|_| "factor must be a number")?;
    let dz: f32 = args
        .get(3)
        .map(|s| s.parse().map_err(|_| "dz must be a number"))
        .transpose()?
        .unwrap_or(0.0);
    let report = ds2_modkit::characters::mesh_head::scale_heads(Path::new(dec), factor, dz)?;
    for line in &report {
        println!("{line}");
    }
    Ok(())
}

fn head_hide(args: &[String]) -> ds2_modkit::Result<()> {
    let dec = args
        .get(1)
        .ok_or("usage: ds2-modkit head-hide <death.dec>")?;
    let report = ds2_modkit::characters::mesh_head::hide_stock_head(Path::new(dec))?;
    for line in &report {
        println!("{line}");
    }
    Ok(())
}
