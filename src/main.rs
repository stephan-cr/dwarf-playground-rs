//! A simple example of parsing `.debug_info`.

#![warn(rust_2018_idioms)]
#![warn(clippy::pedantic)]

use gimli::read::{AttributeValue, EvaluationResult};
use object::{Object, ObjectSection};
use std::fs::File;
use std::io::Write;
use std::{borrow, env, error, fs};

use clap::{crate_name, crate_version, Arg, Command};

fn main() -> Result<(), Box<dyn error::Error>> {
    let matches = Command::new(crate_name!())
        .version(crate_version!())
        .arg(Arg::new("pe").num_args(1))
        .get_matches();

    if !matches.contains_id("pe") {
        for path in env::args().skip(1) {
            let file = fs::File::open(&path)?;
            let mmap = unsafe { memmap2::Mmap::map(&file)? };
            let object = object::File::parse(&*mmap)?;
            let endian = if object.is_little_endian() {
                gimli::RunTimeEndian::Little
            } else {
                gimli::RunTimeEndian::Big
            };
            dump_file(&object, endian)?;
        }
    } else if let Some(value) = matches.get_one::<String>("pe") {
        let file = fs::File::open(&value)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let object = object::File::parse(&*mmap)?;
        println!("{:?}", object.format());
        let endian = if object.is_little_endian() {
            gimli::RunTimeEndian::Little
        } else {
            gimli::RunTimeEndian::Big
        };
        dump_file(&object, endian)?;
    }

    Ok(())
}

fn dump_file(object: &object::File<'_>, endian: gimli::RunTimeEndian) -> Result<(), gimli::Error> {
    // Load a section and return as `Cow<[u8]>`.
    let load_section = |id: gimli::SectionId| -> Result<borrow::Cow<'_, [u8]>, gimli::Error> {
        match object.section_by_name(dbg!(id.name())) {
            Some(ref section) => Ok(section
                .uncompressed_data()
                .unwrap_or(borrow::Cow::Borrowed(&[][..]))),
            None => Ok(borrow::Cow::Borrowed(&[][..])),
        }
    };

    // Load all of the sections.
    let dwarf_cow = gimli::Dwarf::load(&load_section)?;

    // Borrow a `Cow<[u8]>` to create an `EndianSlice`.
    let borrow_section: &dyn for<'a> Fn(
        &'a borrow::Cow<'_, [u8]>,
    ) -> gimli::EndianSlice<'a, gimli::RunTimeEndian> =
        &|section| gimli::EndianSlice::new(section, endian);

    // Create `EndianSlice`s for all of the sections.
    let dwarf = dwarf_cow.borrow(&borrow_section);

    // Iterate over the compilation units.
    let mut iter = dwarf.units();
    let mut file = File::create("depth_deltas.dat")?;
    while let Some(header) = iter.next()? {
        println!(
            "Unit at <.debug_info+0x{:x}> version: {}",
            header.offset().as_debug_info_offset().unwrap().0,
            header.version()
        );
        let unit = dwarf.unit(header)?;

        // Iterate over the Debugging Information Entries (DIEs) in the unit.
        let mut depth = 0;
        let mut entries = unit.entries();
        file.write_all(b"--\n")?;
        while let Some((delta_depth, entry)) = entries.next_dfs()? {
            depth += delta_depth;
            file.write_fmt(format_args!("{} {}\n", delta_depth, depth))?;
            println!("<{}><{:x}> {}", depth, entry.offset().0, entry.tag());

            // Iterate over the attributes in the DIE.
            let mut attrs = entry.attrs();
            while let Some(attr) = attrs.next()? {
                match attr.value() {
                    AttributeValue::DebugStrRef(offset) => {
                        println!(
                            "offset = {} {:?}",
                            offset.0,
                            dwarf.string(offset)?.escape_ascii()
                        );
                    }
                    AttributeValue::Exprloc(expr) => {
                        let mut eval = expr.evaluation(unit.encoding());
                        let result = match eval.evaluate()? {
                            EvaluationResult::Complete => eval.result(),
                            EvaluationResult::RequiresRelocatedAddress(addr) => {
                                dbg!(eval.resume_with_relocated_address(addr)?);
                                eval.result()
                            }
                            _otherwise => vec![],
                        };
                        println!("{:?}", result);
                    }
                    AttributeValue::String(_) => {
                        let x = dwarf.attr_string(&unit, attr.value())?;
                        println!("string => {}", x.to_string()?);
                    }
                    AttributeValue::UnitRef(x) => {
                        unit.entry(x)?;
                    }
                    _ => (),
                }
                println!("   {}: {:?}", attr.name(), attr.value());
            }
        }
    }

    Ok(())
}
