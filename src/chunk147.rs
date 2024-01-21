use fastnbt::{ByteArray, IntArray, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Eq, Hash, PartialEq)]
pub struct Block {
    id: i32,
    data: Option<i8>,
}

impl Block {
    pub fn new(id: i32, data: Option<i8>) -> Self {
        Self { id, data }
    }

    pub fn to_i8(&self) -> i8 {
        (self.id & 255) as i8
    }

    pub fn add_to_i8(&self) -> i8 {
        ((self.id & 0b111100000000) >> 8) as i8
    }

    pub fn from_i8(block: i8) -> Self {
        let mut id = 0 as i32;

        id |= (block & 0b01111111) as i32;
        id |= (((block >> 7) as i32) & 0b1) << 7;

        Self { id, data: None }
    }
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
    pub fn mut_tile_entities(&mut self) -> &mut Vec<TileEntity> {
        &mut self.tile_entities
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
        let mut block = self.blocks[block_pos] as i32;
        if let Some(added) = &self.add {
            block += ((added[block_pos / 2] as i32) >> (4 * (block_pos % 2))) << 8;
        }
        let mut data = None;
        if let Some(data_arr) = &self.data {
            let d = if block_pos % 2 == 0 {
                data_arr[block_pos / 2] & 0x0F
            } else {
                (data_arr[block_pos / 2] >> 4) & 0x0F
            };
            data = Some(d);
        }

        Block { id: block, data }
    }

    pub fn replace_all_blocks(&mut self, conversion_map: Arc<RwLock<HashMap<Block, Block>>>) {
        if let Some(added) = &self.add {
            for (idx, block) in self.blocks.iter_mut().enumerate() {
                let mut b = Block::from_i8(*block);
                let d = if idx % 2 == 0 {
                    added[idx / 2] & 0x0F
                } else {
                    (added[idx / 2] >> 4) & 0x0F
                };
                b.id += (d as i32) << 8;
                if let Some(data_arr) = &self.data {
                    let d = if idx % 2 == 0 {
                        data_arr[idx / 2] & 0x0F
                    } else {
                        (data_arr[idx / 2] >> 4) & 0x0F
                    };
                    b.data = Some(d);
                }
                if let Some(new_block) = conversion_map.read().unwrap().get(&b) {
                    self.replace_block(idx, new_block);
                } else {
                    b.data = None;
                    if let Some(new_block) = conversion_map.read().unwrap().get(&b) {
                        self.replace_block(idx, new_block);
                    }
                }
            }
        }
    }

    fn replace_block(&mut self, idx: usize, new_block: &Block) {
        if let Some(new_data) = new_block.data {
            if let Some(data_arr) = &self.data {
                let (mask, shift) = if idx % 2 == 0 {
                    (0b1111, 0)
                } else {
                    (-0b1110000, 4)
                };
                data_arr[idx / 2] &= !mask;
                data_arr[idx / 2] |= new_data << shift;
            }
        }

        if let Some(added) = &self.add {
            let (mask, shift) = if idx % 2 == 0 {
                (0b1111, 0)
            } else {
                (-0b1110000, 4)
            };
            self.blocks[idx] = new_block.to_i8();
            added[idx / 2] &= !mask;
            added[idx / 2] |= new_block.add_to_i8() << shift;
        }
    }
}
