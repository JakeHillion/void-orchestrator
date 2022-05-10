use crate::{Result, Specification};

use std::fs::File;
use std::path::Path;

use bincode::Options;
use object::endian::Endianness;
use object::read::ReadCache;
use object::read::{Object, ObjectSection};
use object::write::{StandardSegment, StreamingBuffer};
use object::SectionKind;

const SPECIFICATION_SECTION_NAME: &str = "void_specification";

pub(crate) fn pack_binary(binary: &Path, spec: &Specification, output: &Path) -> Result<()> {
    let binary = File::open(binary)?;
    let binary = ReadCache::new(binary);

    let output = File::create(output)?;
    let mut output = StreamingBuffer::new(output);

    let input_object = object::File::parse(&binary)?;

    let format = input_object.format();
    let architecture = input_object.architecture();
    let endianness = if input_object.is_little_endian() {
        Endianness::Little
    } else {
        Endianness::Big
    };

    let mut output_object = object::write::Object::new(format, architecture, endianness);

    for input_section in input_object.sections() {
        let output_section = output_object.add_section(
            input_section.segment_name_bytes()?.unwrap_or(&[]).to_vec(),
            input_section.name_bytes()?.to_vec(),
            input_section.kind(),
        );

        output_object.set_section_data(output_section, input_section.data()?, 0);
    }

    let spec_section = output_object.add_section(
        output_object.segment_name(StandardSegment::Debug).to_vec(),
        SPECIFICATION_SECTION_NAME.to_string().into(),
        SectionKind::Other,
    );

    let spec = bincode_options().serialize(spec)?;
    output_object.set_section_data(spec_section, spec, 0);

    output_object.emit(&mut output)?;
    Ok(())
}

pub(crate) fn extract_specification(binary: &Path) -> Result<Option<Specification>> {
    let binary = File::open(binary)?;
    let binary = ReadCache::new(binary);
    let input_object = object::File::parse(&binary)?;

    let spec_section = if let Some(s) = input_object.section_by_name(SPECIFICATION_SECTION_NAME) {
        s
    } else {
        return Ok(None);
    };

    let spec_data = spec_section.data()?;

    Ok(Some(bincode_options().deserialize(spec_data)?))
}

fn bincode_options() -> impl bincode::Options {
    bincode::DefaultOptions::new()
}
