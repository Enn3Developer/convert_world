use fastnbt::{ByteArray, IntArray, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Block = i32;

pub fn convert_block_to_i8(block: &Block) -> i8 {
    (block & 255) as i8
}

pub fn convert_i8_to_block(block: i8) -> Block {
    let mut b = 0 as Block;

    b |= (block & 0b01111111) as Block;
    b |= (((block >> 7) as Block) & 0b1) << 7;

    b
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Chunk {
    level: Level,
}

impl Chunk {
    pub fn level(&self) -> &Level {
        &self.level
    }
    pub fn mut_level(&mut self) -> &mut Level {
        &mut self.level
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Level {
    biomes: Option<ByteArray>,
    tile_entities: Vec<TileEntity>,
    height_map: IntArray,
    sections: Vec<Section>,
    last_update: i64,
    #[serde(rename = "xPos")]
    x_pos: i32,
    #[serde(rename = "zPos")]
    z_pos: i32,
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl Level {
    pub fn last_update(&self) -> i64 {
        self.last_update
    }
    pub fn biomes(&self) -> Option<&ByteArray> {
        self.biomes.as_ref()
    }
    pub fn tile_entities(&self) -> &Vec<TileEntity> {
        &self.tile_entities
    }
    pub fn height_map(&self) -> &IntArray {
        &self.height_map
    }
    pub fn sections(&self) -> &Vec<Section> {
        &self.sections
    }
    pub fn mut_sections(&mut self) -> &mut Vec<Section> {
        &mut self.sections
    }
    pub fn x_pos(&self) -> i32 {
        self.x_pos
    }
    pub fn z_pos(&self) -> i32 {
        self.z_pos
    }
    pub fn other(&self) -> &HashMap<String, Value> {
        &self.other
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct TileEntity {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "x")]
    x: i32,
    #[serde(rename = "y")]
    y: i32,
    #[serde(rename = "z")]
    z: i32,

    // GUI Containers
    custom_name: Option<String>,
    // Beacon
    levels: Option<i32>,
    primary: Option<i32>,
    secondary: Option<i32>,
    // Cauldron / Chest / Furnace / Hopper / Trap
    items: Option<Vec<Value>>,
    // Cauldron
    brew_time: Option<i32>,
    // Control
    command: Option<String>,
    // Furnace
    burn_time: Option<i16>,
    cook_time: Option<i16>,
    // Mob Spawner
    entity_id: Option<String>,
    spawn_count: Option<i16>,
    spawn_range: Option<i16>,
    spawn_data: Option<Value>,
    spawn_potentials: Option<Vec<Value>>,
    delay: Option<i16>,
    min_spawn_delay: Option<i16>,
    max_spawn_delay: Option<i16>,
    max_nearby_entities: Option<i16>,
    required_player_range: Option<i16>,
    max_experience: Option<i32>,
    remaining_experience: Option<i32>,
    experience_regen_tick: Option<i32>,
    experience_regen_rate: Option<i32>,
    experience_regen_amount: Option<i32>,
    // Music
    #[serde(rename = "note")]
    note: Option<i8>,
    // Piston
    #[serde(rename = "blockId")]
    block_id: Option<i32>,
    #[serde(rename = "blockData")]
    block_data: Option<i32>,
    #[serde(rename = "facing")]
    facing: Option<i32>,
    #[serde(rename = "progress")]
    progress: Option<f32>,
    #[serde(rename = "extending")]
    extending: Option<i8>,
    // Record Player
    record: Option<i32>,
    record_item: Option<Value>,
    // Sign
    text1: Option<String>,
    text2: Option<String>,
    text3: Option<String>,
    text4: Option<String>,
    // Skull
    skull_type: Option<i8>,
    extra_type: Option<String>,
    rot: Option<i8>,
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl TileEntity {
    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn other(&self) -> &HashMap<String, Value> {
        &self.other
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Section {
    blocks: ByteArray,
    add: Option<ByteArray>,
    y: i8,
    data: Option<ByteArray>,
    #[serde(flatten)]
    other: HashMap<String, Value>,
}

impl Section {
    pub fn y(&self) -> i8 {
        self.y
    }

    pub fn blocks(&self) -> &ByteArray {
        &self.blocks
    }

    pub fn add(&self) -> Option<&ByteArray> {
        self.add.as_ref()
    }

    pub fn other(&self) -> &HashMap<String, Value> {
        &self.other
    }

    pub fn block(&self, x: i32, y: i32, z: i32) -> Block {
        let block_pos = (y * 16 * 16 + z * 16 + x) as usize;
        let mut block = self.blocks[block_pos] as Block;
        if let Some(added) = &self.add {
            block += ((added[block_pos / 2] as i32) >> (4 * (block_pos % 2))) << 8;
        }
        block
    }

    fn replace_all_simple(&mut self, old: i8, new: i8) {
        for block in self.blocks.iter_mut() {
            if *block == old {
                *block = new;
            }
        }
    }

    pub fn replace_all(&mut self, old: Block, new: Block) {
        if old < 256 {
            self.replace_all_simple(convert_block_to_i8(&old), convert_block_to_i8(&new));
        } else {
            if let Some(added) = &self.add {
                for (idx, block) in self.blocks.iter_mut().enumerate() {
                    let b = convert_i8_to_block(*block);
                    let d = if idx % 2 == 0 {
                        added[idx / 2] & 0x0F
                    } else {
                        (added[idx / 2] >> 4) & 0x0F
                    };
                    let id = b + ((d as Block) << 8);
                    if id == old {
                        *block = convert_block_to_i8(&new);
                    }
                }
            }
        }
    }

    pub fn replace_all_data_simple(&mut self, old: i8, new: i8, new_data: i8) {
        for (idx, block) in self.blocks.iter_mut().enumerate() {
            if *block == old {
                *block = new;
                if let Some(data) = &mut self.data {
                    let (mask, shift) = if idx % 2 == 0 {
                        (0b1111, 0)
                    } else {
                        (-0b1110000, 4)
                    };
                    data[idx / 2] &= !mask;
                    data[idx / 2] |= new_data << shift;
                }
            }
        }
    }

    pub fn replace_all_data(&mut self, old: Block, new: Block, new_data: i8) {
        if old < 256 {
            self.replace_all_data_simple(
                convert_block_to_i8(&old),
                convert_block_to_i8(&new),
                new_data,
            );
        } else {
            if let Some(added) = &mut self.add {
                for (idx, block) in self.blocks.iter_mut().enumerate() {
                    let b = convert_i8_to_block(*block);
                    let d = if idx % 2 == 0 {
                        added[idx / 2] & 0x0F
                    } else {
                        (added[idx / 2] >> 4) & 0x0F
                    };
                    let id = b + ((d as Block) << 8);
                    if id == old {
                        *block = convert_block_to_i8(&new);
                        added[idx / 2] -= d << (4 * (idx % 2));
                        if let Some(data) = &mut self.data {
                            let (mask, shift) = if idx % 2 == 0 {
                                (0b1111, 0)
                            } else {
                                (-0b1110000, 4)
                            };
                            data[idx / 2] &= !mask;
                            data[idx / 2] |= new_data << shift;
                            // data[idx / 2].set_bits(range, new_data);
                        }
                    }
                }
            }
        }
    }
}
