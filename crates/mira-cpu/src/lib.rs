pub mod cpu;
pub mod isa;

pub use cpu::{
    assemble_j_imm, reg_index, sext, Cpu, StepOutcome, Trap, DEFAULT_RAM_SIZE, RESET_PC,
};

#[cfg(test)]
mod tests {
    use super::isa::*;
    use super::*;

    fn program(words: &[u32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(words.len() * 4);
        for word in words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    fn cpu_with(words: &[u32]) -> Cpu {
        let mut cpu = Cpu::new();
        cpu.load_program(&program(words), RESET_PC).unwrap();
        cpu
    }

    fn run(cpu: &mut Cpu) {
        for _ in 0..10_000 {
            match cpu.step().unwrap() {
                StepOutcome::Halt => return,
                StepOutcome::Continue => {}
            }
        }
        panic!("program did not halt");
    }

    #[test]
    fn addi_writes_register() {
        let mut cpu = cpu_with(&[addi(1, 0, 42), ebreak()]);
        run(&mut cpu);
        assert_eq!(cpu.reg(1), 42);
    }

    #[test]
    fn mov_pseudo_writes_negative_immediate() {
        let mut cpu = cpu_with(&[addi(5, 0, -1), ebreak()]);
        run(&mut cpu);
        assert_eq!(cpu.reg(5), u64::MAX);
    }

    #[test]
    fn add_and_sub() {
        let mut cpu = cpu_with(&[
            addi(1, 0, 5),
            addi(2, 0, 7),
            add(3, 1, 2),
            sub(4, 3, 1),
            ebreak(),
        ]);
        run(&mut cpu);
        assert_eq!(cpu.reg(3), 12);
        assert_eq!(cpu.reg(4), 7);
    }

    #[test]
    fn r0_is_always_zero() {
        let mut cpu = cpu_with(&[addi(0, 0, 99), addi(1, 0, 1), add(0, 1, 1), ebreak()]);
        run(&mut cpu);
        assert_eq!(cpu.reg(0), 0);
        assert_eq!(cpu.regs[0], 0);
    }

    #[test]
    fn jal_jumps_forward_and_links() {
        let mut cpu = cpu_with(&[jal(1, 8), addi(2, 0, 111), addi(3, 0, 222), ebreak()]);
        run(&mut cpu);
        assert_eq!(cpu.reg(1), 4);
        assert_eq!(cpu.reg(2), 0);
        assert_eq!(cpu.reg(3), 222);
    }

    #[test]
    fn jal_to_r0_does_not_link() {
        let mut cpu = cpu_with(&[jal(0, 8), addi(2, 0, 111), addi(3, 0, 1), ebreak()]);
        run(&mut cpu);
        assert_eq!(cpu.regs[0], 0);
        assert_eq!(cpu.reg(2), 0);
        assert_eq!(cpu.reg(3), 1);
    }

    #[test]
    fn ebreak_halts() {
        let mut cpu = cpu_with(&[addi(1, 0, 1), ebreak(), addi(1, 0, 2)]);
        run(&mut cpu);
        assert_eq!(cpu.reg(1), 1);
        assert_eq!(cpu.pc, 4);
    }

    #[test]
    fn memory_roundtrip() {
        let mut cpu = Cpu::new();
        cpu.write_u64(0x100, 0xdead_beef_cafe_babe).unwrap();
        assert_eq!(cpu.read_u64(0x100).unwrap(), 0xdead_beef_cafe_babe);
        assert_eq!(cpu.read_u8(0x100).unwrap(), 0xbe);
        cpu.write_u32(0x200, 0x1234_5678).unwrap();
        assert_eq!(cpu.read_u32(0x200).unwrap(), 0x1234_5678);
        assert_eq!(cpu.read_u8(0x201).unwrap(), 0x56);
    }

    #[test]
    fn bus_error_out_of_range() {
        let cpu = Cpu::new();
        assert_eq!(
            cpu.read_u32(0x1000_0000),
            Err(Trap::BusError {
                addr: 0x1000_0000,
                len: 4
            })
        );
    }

    #[test]
    fn load_program_out_of_range() {
        let mut cpu = Cpu::new();
        let data = vec![0u8; 4];
        let addr = cpu.memory.len() as u64 - 2;
        assert_eq!(
            cpu.load_program(&data, addr),
            Err(Trap::BusError { addr, len: 4 })
        );
    }

    #[test]
    fn misaligned_pc_traps() {
        let mut cpu = Cpu::new();
        cpu.pc = 2;
        assert_eq!(cpu.step(), Err(Trap::MisalignedPc(2)));
    }

    #[test]
    fn unknown_opcode_traps() {
        let mut cpu = cpu_with(&[0x0000_0000]);
        assert_eq!(
            cpu.step(),
            Err(Trap::IllegalInstruction { instr: 0, pc: 0 })
        );
    }

    #[test]
    fn high_register_field_traps() {
        let instr = addi(16, 0, 1);
        let mut cpu = cpu_with(&[instr]);
        assert_eq!(cpu.step(), Err(Trap::IllegalInstruction { instr, pc: 0 }));
    }

    #[test]
    fn ecall_is_unimplemented_for_now() {
        let instr = ecall();
        let mut cpu = cpu_with(&[instr]);
        assert_eq!(cpu.step(), Err(Trap::Unimplemented { instr, pc: 0 }));
    }

    #[test]
    fn addi_sign_extends_immediate() {
        let mut cpu = cpu_with(&[addi(1, 1, -2), ebreak()]);
        run(&mut cpu);
        assert_eq!(cpu.reg(1) as i64, -2);
    }

    #[test]
    fn jimm_roundtrip() {
        for imm in [-8i32, -4, 0, 4, 8, 16, 1024, -1024] {
            let instr = jal(1, imm);
            let decoded = sext(assemble_j_imm(instr), 21) as i32;
            assert_eq!(decoded, imm);
        }
    }
}
