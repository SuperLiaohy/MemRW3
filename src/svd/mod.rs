use std::path::Path;

use anyhow::{Context, Result};
use svd_parser::svd::{
    Access, ClusterInfo, FieldInfo, MaybeArray, PeripheralInfo, RegisterCluster, RegisterInfo,
};

#[derive(Debug, Clone)]
pub struct SvdTree {
    pub device_name: String,
    pub peripherals: Vec<SvdPeripheral>,
}

#[derive(Debug, Clone)]
pub struct SvdPeripheral {
    pub name: String,
    pub description: Option<String>,
    pub base_address: u64,
    pub registers: Vec<SvdRegister>,
}

#[derive(Debug, Clone)]
pub struct SvdRegister {
    pub name: String,
    pub description: Option<String>,
    pub address: u64,
    pub size_bits: Option<u32>,
    pub access: Option<String>,
    pub reset_value: Option<u64>,
    pub fields: Vec<SvdField>,
}

#[derive(Debug, Clone)]
pub struct SvdField {
    pub name: String,
    pub description: Option<String>,
    pub bit_offset: u32,
    pub bit_width: u32,
    pub access: Option<String>,
}

pub fn load_svd(path: &Path) -> Result<SvdTree> {
    let xml = std::fs::read_to_string(path)
        .with_context(|| format!("读取 SVD 失败: {}", path.display()))?;
    parse_svd(&xml).with_context(|| format!("解析 SVD 失败: {}", path.display()))
}

pub fn parse_svd(xml: &str) -> Result<SvdTree> {
    let config = svd_parser::Config::default()
        .validate_level(svd_parser::ValidateLevel::Weak)
        .expand_properties(true)
        .expand(true);
    let device = svd_parser::parse_with_config(xml, &config)?;
    let mut peripherals = Vec::new();
    for peripheral in &device.peripherals {
        match peripheral {
            MaybeArray::Single(info) => peripherals.push(convert_peripheral(info)?),
            MaybeArray::Array(info, dim) => {
                for instance in svd_parser::svd::peripheral::expand(info, dim) {
                    peripherals.push(convert_peripheral(&instance)?);
                }
            }
        }
    }
    peripherals.sort_by(|left, right| {
        left.base_address
            .cmp(&right.base_address)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(SvdTree {
        device_name: device.name,
        peripherals,
    })
}

fn convert_peripheral(peripheral: &PeripheralInfo) -> Result<SvdPeripheral> {
    let mut registers = Vec::new();
    if let Some(children) = &peripheral.registers {
        collect_registers(children, peripheral.base_address, "", &mut registers)?;
    }
    registers.sort_by(|left, right| {
        left.address
            .cmp(&right.address)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(SvdPeripheral {
        name: peripheral.name.clone(),
        description: peripheral.description.clone(),
        base_address: peripheral.base_address,
        registers,
    })
}

fn collect_registers(
    children: &[RegisterCluster],
    base_address: u64,
    prefix: &str,
    output: &mut Vec<SvdRegister>,
) -> Result<()> {
    for child in children {
        match child {
            RegisterCluster::Register(register) => match register {
                MaybeArray::Single(info) => {
                    output.push(convert_register(info, base_address, prefix)?);
                }
                MaybeArray::Array(info, dim) => {
                    for instance in svd_parser::svd::register::expand(info, dim) {
                        output.push(convert_register(&instance, base_address, prefix)?);
                    }
                }
            },
            RegisterCluster::Cluster(cluster) => match cluster {
                MaybeArray::Single(info) => {
                    collect_cluster(info, base_address, prefix, output)?;
                }
                MaybeArray::Array(info, dim) => {
                    for instance in svd_parser::svd::cluster::expand(info, dim) {
                        collect_cluster(&instance, base_address, prefix, output)?;
                    }
                }
            },
        }
    }
    Ok(())
}

fn collect_cluster(
    cluster: &ClusterInfo,
    parent_address: u64,
    prefix: &str,
    output: &mut Vec<SvdRegister>,
) -> Result<()> {
    let address = parent_address
        .checked_add(u64::from(cluster.address_offset))
        .ok_or_else(|| anyhow::anyhow!("寄存器簇 {} 地址溢出", cluster.name))?;
    let prefix = format!("{prefix}{}.", cluster.name);
    collect_registers(&cluster.children, address, &prefix, output)
}

fn convert_register(
    register: &RegisterInfo,
    base_address: u64,
    prefix: &str,
) -> Result<SvdRegister> {
    let address = base_address
        .checked_add(u64::from(register.address_offset))
        .ok_or_else(|| anyhow::anyhow!("寄存器 {} 地址溢出", register.name))?;
    let mut fields = Vec::new();
    if let Some(source_fields) = &register.fields {
        for field in source_fields {
            match field {
                MaybeArray::Single(info) => {
                    fields.push(convert_field(info, register.properties.access))
                }
                MaybeArray::Array(info, dim) => {
                    fields.extend(
                        svd_parser::svd::field::expand(info, dim)
                            .map(|instance| convert_field(&instance, register.properties.access)),
                    );
                }
            }
        }
    }
    fields.sort_by_key(|field| field.bit_offset);
    Ok(SvdRegister {
        name: format!("{prefix}{}", register.name),
        description: register.description.clone(),
        address,
        size_bits: register.properties.size,
        access: access_label(register.properties.access),
        reset_value: register.properties.reset_value,
        fields,
    })
}

fn convert_field(field: &FieldInfo, inherited_access: Option<Access>) -> SvdField {
    SvdField {
        name: field.name.clone(),
        description: field.description.clone(),
        bit_offset: field.bit_offset(),
        bit_width: field.bit_width(),
        access: access_label(field.access.or(inherited_access)),
    }
}

fn access_label(access: Option<Access>) -> Option<String> {
    access.map(|access| access.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::parse_svd;

    const ARRAY_SVD: &str = r#"
<device schemaVersion="1.3">
  <name>TEST</name><version>1.0</version><description>Test</description>
  <addressUnitBits>8</addressUnitBits><width>32</width>
  <peripherals><peripheral><name>TIMER</name><description>Timer</description>
    <baseAddress>0x40000000</baseAddress><registers>
      <register><dim>2</dim><dimIncrement>4</dimIncrement><dimIndex>0,1</dimIndex>
        <name>CH%s</name><description>Channel</description><addressOffset>0x10</addressOffset>
        <size>32</size><access>read-write</access><resetValue>0</resetValue>
        <fields><field><name>EN</name><bitOffset>0</bitOffset><bitWidth>1</bitWidth></field></fields>
      </register>
    </registers></peripheral></peripherals>
</device>"#;

    const DERIVED_SVD: &str = r#"
<device schemaVersion="1.3">
  <name>TEST</name><version>1.0</version><description>Test</description>
  <addressUnitBits>8</addressUnitBits><width>32</width>
  <peripherals><peripheral><name>GPIO</name><baseAddress>0x50000000</baseAddress>
    <registers>
      <register><name>BASE</name><addressOffset>0</addressOffset><size>32</size>
        <access>read-write</access>
        <fields><field><name>VALUE</name><bitOffset>0</bitOffset><bitWidth>8</bitWidth></field></fields>
      </register>
      <register derivedFrom="BASE"><name>COPY</name><addressOffset>4</addressOffset></register>
    </registers>
  </peripheral></peripherals>
</device>"#;

    #[test]
    fn expands_register_arrays_and_computes_absolute_addresses() {
        let tree = parse_svd(ARRAY_SVD).unwrap();
        let registers = &tree.peripherals[0].registers;
        assert_eq!(registers.len(), 2);
        assert_eq!(registers[0].name, "CH0");
        assert_eq!(registers[0].address, 0x4000_0010);
        assert_eq!(registers[1].name, "CH1");
        assert_eq!(registers[1].address, 0x4000_0014);
        assert_eq!(registers[0].fields[0].bit_width, 1);
    }

    #[test]
    fn rejects_invalid_xml() {
        assert!(parse_svd("<device>").is_err());
    }

    #[test]
    fn resolves_derived_register_fields() {
        let tree = parse_svd(DERIVED_SVD).unwrap();
        let copy = tree.peripherals[0]
            .registers
            .iter()
            .find(|register| register.name == "COPY")
            .unwrap();
        assert_eq!(copy.address, 0x5000_0004);
        assert_eq!(copy.size_bits, Some(32));
        assert_eq!(copy.fields[0].name, "VALUE");
    }
}
