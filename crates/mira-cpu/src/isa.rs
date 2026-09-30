pub const OP_LUI: u32 = 0x37;
pub const OP_AUIPC: u32 = 0x17;
pub const OP_JAL: u32 = 0x6f;
pub const OP_JALR: u32 = 0x67;
pub const OP_BRANCH: u32 = 0x63;
pub const OP_LOAD: u32 = 0x03;
pub const OP_STORE: u32 = 0x23;
pub const OP_IMM: u32 = 0x13;
pub const OP: u32 = 0x33;
pub const OP_SYSTEM: u32 = 0x73;

pub const FUNCT3_ADDI: u32 = 0x0;
pub const FUNCT3_ADD_SUB: u32 = 0x0;
pub const FUNCT7_ADD: u32 = 0x00;
pub const FUNCT7_SUB: u32 = 0x20;

pub const PRIV_ECALL: u32 = 0x000;
pub const PRIV_EBREAK: u32 = 0x001;

pub fn encode_r(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

pub fn encode_i(imm: i32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    ((imm as u32 & 0xfff) << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

pub fn encode_j(imm: i32, rd: u32, opcode: u32) -> u32 {
    let imm = imm as u32;
    let b20 = (imm >> 20) & 0x1;
    let b10_1 = (imm >> 1) & 0x3ff;
    let b11 = (imm >> 11) & 0x1;
    let b19_12 = (imm >> 12) & 0xff;
    (b20 << 31) | (b10_1 << 21) | (b11 << 20) | (b19_12 << 12) | (rd << 7) | opcode
}

pub fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    encode_i(imm, rs1, FUNCT3_ADDI, rd, OP_IMM)
}

pub fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    encode_r(FUNCT7_ADD, rs2, rs1, FUNCT3_ADD_SUB, rd, OP)
}

pub fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
    encode_r(FUNCT7_SUB, rs2, rs1, FUNCT3_ADD_SUB, rd, OP)
}

pub fn jal(rd: u32, imm: i32) -> u32 {
    encode_j(imm, rd, OP_JAL)
}

pub fn ebreak() -> u32 {
    encode_i(PRIV_EBREAK as i32, 0, 0, 0, OP_SYSTEM)
}

pub fn ecall() -> u32 {
    encode_i(PRIV_ECALL as i32, 0, 0, 0, OP_SYSTEM)
}
