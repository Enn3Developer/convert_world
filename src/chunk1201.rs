use crate::chunk147;
use bit_field::BitField;
use fastnbt::{LongArray, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct Chunk {
    data_version: Option<i32>,
    #[serde(rename = "xPos")]
    x_pos: Option<i32>,
    #[serde(rename = "yPos")]
    y_pos: Option<i32>,
    #[serde(rename = "zPos")]
    z_pos: Option<i32>,
    status: Option<String>,
    last_update: Option<i64>,
    #[serde(rename = "sections")]
    sections: Option<Vec<Section>>,
    #[serde(rename = "block_entities")]
    block_entities: Option<Vec<BlockEntity>>,
    inhabited_time: Option<i64>,

    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl Chunk {
    pub fn block_entities(&self) -> Option<&Vec<BlockEntity>> {
        self.block_entities.as_ref()
    }

    pub fn sections(&self) -> Option<&Vec<Section>> {
        self.sections.as_ref()
    }

    pub fn mut_sections(&mut self) -> Option<&mut Vec<Section>> {
        self.sections.as_mut()
    }

    pub fn convert_old(
        old: &chunk147::Chunk,
        conversion_map: &HashMap<chunk147::Block, String>,
    ) -> Self {
        let sections = old
            .level()
            .sections()
            .iter()
            .map(|sec| Section::convert_old(sec, conversion_map))
            .collect::<Vec<_>>();
        Self {
            last_update: Some(old.level().last_update()),
            x_pos: Some(old.level().x_pos()),
            z_pos: Some(old.level().z_pos()),
            sections: Some(sections),
            ..Default::default()
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Section {
    y: i8,
    #[serde(rename = "block_states")]
    block_states: Option<BlockStates>,
    #[serde(rename = "biomes")]
    biomes: Option<Biomes>,
}

impl Section {
    pub fn block_states(&self) -> Option<&BlockStates> {
        self.block_states.as_ref()
    }

    pub fn mut_block_states(&mut self) -> Option<&mut BlockStates> {
        self.block_states.as_mut()
    }

    pub fn convert_old(
        old: &chunk147::Section,
        conversion_map: &HashMap<chunk147::Block, String>,
    ) -> Self {
        let default = String::from("minecraft:air");
        let mut palette = vec![Block::new(default.clone(), None)];
        for (idx, block) in old.blocks().iter().enumerate() {
            if let Some(added) = old.add() {
                let id = (*block as i32) + (((added[idx / 2] as i32) >> (4 * (idx % 2))) << 8);
                let new_id = conversion_map
                    .get(&(id as chunk147::Block))
                    .unwrap_or(&default);
                let new_block = Block::new(new_id.clone(), None);
                if !palette.contains(&new_block) {
                    palette.push(new_block);
                }
            }
        }
        let mut block_states = BlockStates::new(palette);
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let block = old.block(x, y, z);
                    let converted = conversion_map.get(&block).unwrap_or(&default);
                    let new_block = Block::new(converted.clone(), None);
                    block_states.set_block(&new_block, x, y, z);
                }
            }
        }
        Self {
            y: old.y(),
            block_states: Some(block_states),
            biomes: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Biomes {
    palette: Option<Vec<String>>,
    data: Option<LongArray>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BlockStates {
    palette: Vec<Block>,
    data: Option<LongArray>,
}

impl BlockStates {
    pub fn new(palette: Vec<Block>) -> Self {
        if palette.len() == 1 {
            return Self {
                palette,
                data: None,
            };
        }
        let bits = ((usize::BITS - (palette.len() - 1).leading_zeros()) as usize).max(4);
        let values_per_64bits = 64 / bits;
        let max = (15 * 16 * 16 + 15 * 16 + 15) / values_per_64bits;
        let data = vec![0; max + 1];

        Self {
            palette,
            data: Some(LongArray::new(data)),
        }
    }

    pub fn palette(&self) -> &Vec<Block> {
        &self.palette
    }

    pub fn data(&self) -> Option<&LongArray> {
        self.data.as_ref()
    }

    pub fn block(&self, x: i32, y: i32, z: i32) -> &Block {
        if self.data.is_none() {
            self.palette.first().unwrap()
        } else {
            let data = self.data.as_ref().unwrap();
            let block_pos = (y * 16 * 16 + z * 16 + x) as usize;
            let bits = ((usize::BITS - (self.palette.len() - 1).leading_zeros()) as usize).max(4);
            let values_per_64bits = 64 / bits;

            let long_index = block_pos / values_per_64bits;
            let inter_index = block_pos % values_per_64bits;
            let range = inter_index * bits..(inter_index + 1) * bits;
            let long = data[long_index] as u64;
            let palette_index = long.get_bits(range);

            &self.palette[palette_index as usize]
        }
    }

    pub fn set_block(&mut self, block: &Block, x: i32, y: i32, z: i32) {
        let mut idx = self.palette_contains(block);
        if idx == -1 {
            idx = self.add_to_palette(block.clone());
            // TODO: recreate data
        }

        let block_pos = (y * 16 * 16 + z * 16 + x) as usize;
        let bits = ((usize::BITS - (self.palette.len() - 1).leading_zeros()) as usize).max(4);
        let values_per_64bits = 64 / bits;

        let long_index = block_pos / values_per_64bits;
        let inter_index = block_pos % values_per_64bits;
        let range = inter_index * bits..(inter_index + 1) * bits;

        while self.data.is_some() && self.data.clone().unwrap().get(long_index).is_none() {
            let mut tmp = self.data.clone().unwrap().into_inner();
            tmp.push(0);
            self.data = Some(LongArray::new(tmp));
        }

        if let Some(data) = &mut self.data {
            let mut long = data[long_index] as u64;
            long.set_bits(range, idx as u64);
            data[long_index] = long as i64;
        }
    }

    pub fn palette_contains(&self, block: &Block) -> isize {
        for (idx, b) in self.palette.iter().enumerate() {
            if b == block {
                return idx as isize;
            }
        }

        -1
    }

    pub fn add_to_palette(&mut self, block: Block) -> isize {
        self.palette.push(block);
        (self.palette.len() - 1) as isize
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Block {
    name: String,
    properties: Option<Value>,
}

impl Block {
    pub fn new(name: String, properties: Option<Value>) -> Self {
        Self { name, properties }
    }

    pub fn name(&self) -> &String {
        &self.name
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct BlockEntity {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "keepPacked")]
    keep_packed: Option<i8>,
    #[serde(rename = "x")]
    x: i32,
    #[serde(rename = "y")]
    y: i32,
    #[serde(rename = "z")]
    z: i32,

    // GUI / Containers
    custom_name: Option<String>,
    // Items
    // items: Option<Vec<Value>>,
    // // Furnace
    // burn_time: Option<i16>,
    // cook_time: Option<i16>,
    // // Sign
    // #[serde(rename = "is_waxed")]
    // is_waxed: Option<i8>,
    // #[serde(rename = "front_text")]
    // front_text: Option<SignText>,
    // #[serde(rename = "back_text")]
    // back_text: Option<SignText>,
    // // Skull
    // extra_type: Option<String>,
    // // Mob Spawner
    // delay: Option<i16>,
    // max_nearby_entities: Option<i16>,
    // max_spawn_delay: Option<i16>,
    // min_spawn_delay: Option<i16>,
    // required_player_range: Option<i16>,
    // spawn_count: Option<i16>,
    // spawn_range: Option<i16>,
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl BlockEntity {
    pub fn id(&self) -> &String {
        &self.id
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SignText {
    has_glowing_text: i8,
    color: String,
    messages: Vec<String>,
}
