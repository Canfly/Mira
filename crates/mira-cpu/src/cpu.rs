use crate::isa;

pub const DEFAULT_RAM_SIZE: usize = 1024 * 1024;
pub const RESET_PC: u64 = 0;
pub const NUM_REGS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    IllegalInstruction { instr: u32, pc: u64 },
    Unimplemented { instr: u32, pc: u64 },
    MisalignedPc(u64),
    BusError { addr: u64, len: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Continue,
    Halt,
}

#[derive(Debug, Clone)]
pub struct Cpu {
    pub regs: [u64; NUM_REGS],
    pub pc: u64,
    pub memory: Vec<u8>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self::with_ram_size(DEFAULT_RAM_SIZE)
    }

    pub fn with_ram_size(size: usize) -> Self {
        Self {
            regs: [0; NUM_REGS],
            pc: RESET_PC,
            memory: vec![0; size],
        }
    }

    pub fn load_program(&mut self, data: &[u8], address: u64) -> Result<(), Trap> {
        self.check_range(address, data.len() as u64)?;
        let start = address as usize;
        self.memory[start..start + data.len()].copy_from_slice(data);
        Ok(())
    }

    pub fn reg(&self, index: usize) -> u64 {
        if index == 0 {
            0
        } else {
            self.regs[index]
        }
    }

    pub fn set_reg(&mut self, index: usize, value: u64) {
        if index != 0 {
            self.regs[index] = value;
        }
    }

    pub fn read_u8(&self, addr: u64) -> Result<u8, Trap> {
        self.check_range(addr, 1)?;
        Ok(self.memory[addr as usize])
    }

    pub fn read_u32(&self, addr: u64) -> Result<u32, Trap> {
        self.check_range(addr, 4)?;
        let a = addr as usize;
        Ok(u32::from_le_bytes(
            self.memory[a..a + 4].try_into().unwrap(),
        ))
    }

    pub fn read_u64(&self, addr: u64) -> Result<u64, Trap> {
        self.check_range(addr, 8)?;
        let a = addr as usize;
        Ok(u64::from_le_bytes(
            self.memory[a..a + 8].try_into().unwrap(),
        ))
    }

    pub fn write_u8(&mut self, addr: u64, value: u8) -> Result<(), Trap> {
        self.check_range(addr, 1)?;
        self.memory[addr as usize] = value;
        Ok(())
    }

    pub fn write_u32(&mut self, addr: u64, value: u32) -> Result<(), Trap> {
        self.check_range(addr, 4)?;
        let a = addr as usize;
        self.memory[a..a + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    pub fn write_u64(&mut self, addr: u64, value: u64) -> Result<(), Trap> {
        self.check_range(addr, 8)?;
        let a = addr as usize;
        self.memory[a..a + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn check_range(&self, addr: u64, len: u64) -> Result<(), Trap> {
        let end = addr.checked_add(len).ok_or(Trap::BusError { addr, len })?;
        if end > self.memory.len() as u64 {
            return Err(Trap::BusError { addr, len });
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<StepOutcome, Trap> {
        if self.pc % 4 != 0 {
            return Err(Trap::MisalignedPc(self.pc));
        }
        let pc = self.pc;
        let instr = self.read_u32(pc)?;
        let opcode = instr & 0x7f;

        match opcode {
            isa::OP_IMM => {
                let rd = reg_index(instr, instr >> 7, pc)?;
                let rs1 = reg_index(instr, instr >> 15, pc)?;
                let funct3 = (instr >> 12) & 0x7;
                match funct3 {
                    isa::FUNCT3_ADDI => {
                        let imm = sext(instr >> 20, 12) as u64;
                        let value = self.reg(rs1).wrapping_add(imm);
                        self.set_reg(rd, value);
                    }
                    _ => return Err(Trap::IllegalInstruction { instr, pc }),
                }
            }
            isa::OP => {
                let rd = reg_index(instr, instr >> 7, pc)?;
                let rs1 = reg_index(instr, instr >> 15, pc)?;
                let rs2 = reg_index(instr, instr >> 20, pc)?;
                let funct3 = (instr >> 12) & 0x7;
                let funct7 = (instr >> 25) & 0x7f;
                match (funct3, funct7) {
                    (isa::FUNCT3_ADD_SUB, isa::FUNCT7_ADD) => {
                        let value = self.reg(rs1).wrapping_add(self.reg(rs2));
                        self.set_reg(rd, value);
                    }
                    (isa::FUNCT3_ADD_SUB, isa::FUNCT7_SUB) => {
                        let value = self.reg(rs1).wrapping_sub(self.reg(rs2));
                        self.set_reg(rd, value);
                    }
                    _ => return Err(Trap::IllegalInstruction { instr, pc }),
                }
            }
            isa::OP_JAL => {
                let rd = reg_index(instr, instr >> 7, pc)?;
                let imm = sext(assemble_j_imm(instr), 21) as u64;
                self.set_reg(rd, pc.wrapping_add(4));
                let target = pc.wrapping_add(imm);
                if target % 4 != 0 {
                    return Err(Trap::MisalignedPc(target));
                }
                self.pc = target;
                return Ok(StepOutcome::Continue);
            }
            isa::OP_SYSTEM => {
                let imm = instr >> 20;
                match imm {
                    isa::PRIV_EBREAK => return Ok(StepOutcome::Halt),
                    isa::PRIV_ECALL => return Err(Trap::Unimplemented { instr, pc }),
                    _ => return Err(Trap::IllegalInstruction { instr, pc }),
                }
            }
            _ => return Err(Trap::IllegalInstruction { instr, pc }),
        }

        self.pc = pc.wrapping_add(4);
        Ok(StepOutcome::Continue)
    }
}

pub fn reg_index(instr: u32, field: u32, pc: u64) -> Result<usize, Trap> {
    let field = field & 0x1f;
    if field & 0x10 != 0 {
        return Err(Trap::IllegalInstruction { instr, pc });
    }
    Ok(field as usize)
}

pub fn sext(value: u32, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((value as i64) << shift) >> shift
}

pub fn assemble_j_imm(instr: u32) -> u32 {
    (((instr >> 31) & 0x1) << 20)
        | (((instr >> 21) & 0x3ff) << 1)
        | (((instr >> 20) & 0x1) << 11)
        | (((instr >> 12) & 0xff) << 12)
}
