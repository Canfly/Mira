use std::collections::BTreeMap;
use std::fmt;

use mira_cpu::isa;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmError {
    pub line: usize,
    pub message: String,
}

impl AsmError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "asm error: {}", self.message)
        } else {
            write!(f, "asm error (line {}): {}", self.line, self.message)
        }
    }
}

impl std::error::Error for AsmError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    pub origin: u64,
    pub bytes: Vec<u8>,
    pub symbols: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operand {
    Imm(i64),
    Sym,
}

#[derive(Debug, Clone)]
enum Inst {
    Addi {
        rd: usize,
        rs1: usize,
        imm: i32,
    },
    Add {
        rd: usize,
        rs1: usize,
        rs2: usize,
    },
    Sub {
        rd: usize,
        rs1: usize,
        rs2: usize,
    },
    Jal {
        rd: usize,
        target: Operand,
        symbol: String,
    },
    Ebreak,
    Ecall,
}

#[derive(Debug, Clone)]
enum FragmentKind {
    Data(Vec<u8>),
    Pad(u64),
    Instr(Inst),
}

#[derive(Debug, Clone)]
struct Fragment {
    addr: u64,
    kind: FragmentKind,
}

pub fn assemble(source: &str) -> Result<Assembled, AsmError> {
    assemble_at(source, 0)
}

pub fn assemble_at(source: &str, origin: u64) -> Result<Assembled, AsmError> {
    let mut fragments: Vec<Fragment> = Vec::new();
    let mut symbols: BTreeMap<String, u64> = BTreeMap::new();
    let mut pc = origin;

    for (index, raw) in source.lines().enumerate() {
        let line_no = index + 1;
        let mut rest = strip_comment(raw);
        rest = define_labels(rest, pc, &mut symbols, line_no)?;
        let rest = rest.trim();
        if rest.is_empty() {
            continue;
        }

        let directive = rest
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();

        match directive.as_str() {
            ".org" => {
                let args = tokenize(rest);
                expect_count(&args[1..], 1, line_no, ".org")?;
                let addr = parse_imm(&args[1], line_no)?;
                if addr < 0 {
                    return Err(AsmError::new(line_no, ".org address must be non-negative"));
                }
                let addr = addr as u64;
                if addr < pc {
                    return Err(AsmError::new(
                        line_no,
                        format!(".org 0x{addr:x} goes backwards (current 0x{pc:x})"),
                    ));
                }
                if addr > pc {
                    fragments.push(Fragment {
                        addr: pc,
                        kind: FragmentKind::Pad(addr - pc),
                    });
                    pc = addr;
                }
            }
            ".word" => {
                let args = tokenize(rest);
                if args.len() < 2 {
                    return Err(AsmError::new(line_no, ".word needs at least one value"));
                }
                let mut data = Vec::new();
                for token in &args[1..] {
                    let value = parse_imm(token, line_no)? as i32;
                    data.extend_from_slice(&value.to_le_bytes());
                }
                pc += data.len() as u64;
                fragments.push(Fragment {
                    addr: pc - data.len() as u64,
                    kind: FragmentKind::Data(data),
                });
            }
            ".byte" => {
                let args = tokenize(rest);
                if args.len() < 2 {
                    return Err(AsmError::new(line_no, ".byte needs at least one value"));
                }
                let mut data = Vec::new();
                for token in &args[1..] {
                    let value = parse_imm(token, line_no)?;
                    if !(-128..=255).contains(&value) {
                        return Err(AsmError::new(
                            line_no,
                            format!("byte value out of range: {value}"),
                        ));
                    }
                    data.push(value as u8);
                }
                pc += data.len() as u64;
                fragments.push(Fragment {
                    addr: pc - data.len() as u64,
                    kind: FragmentKind::Data(data),
                });
            }
            ".ascii" | ".asciz" | ".string" => {
                let zero_terminated = directive != ".ascii";
                let mut data = extract_string(rest, line_no)?.into_bytes();
                if zero_terminated {
                    data.push(0);
                }
                pc += data.len() as u64;
                fragments.push(Fragment {
                    addr: pc - data.len() as u64,
                    kind: FragmentKind::Data(data),
                });
            }
            _ => {
                let inst = parse_instruction(rest, line_no)?;
                fragments.push(Fragment {
                    addr: pc,
                    kind: FragmentKind::Instr(inst),
                });
                pc += 4;
            }
        }
    }

    let mut bytes = Vec::new();
    for fragment in &fragments {
        if origin + bytes.len() as u64 != fragment.addr {
            return Err(AsmError::new(
                0,
                "internal error: fragment address mismatch",
            ));
        }
        match &fragment.kind {
            FragmentKind::Data(data) => bytes.extend_from_slice(data),
            FragmentKind::Pad(count) => bytes.resize(bytes.len() + *count as usize, 0),
            FragmentKind::Instr(inst) => {
                let word = encode(inst, fragment.addr, &symbols)?;
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
    }

    Ok(Assembled {
        origin,
        bytes,
        symbols,
    })
}

fn encode(inst: &Inst, addr: u64, symbols: &BTreeMap<String, u64>) -> Result<u32, AsmError> {
    Ok(match inst {
        Inst::Addi { rd, rs1, imm } => isa::addi(*rd as u32, *rs1 as u32, *imm),
        Inst::Add { rd, rs1, rs2 } => isa::add(*rd as u32, *rs1 as u32, *rs2 as u32),
        Inst::Sub { rd, rs1, rs2 } => isa::sub(*rd as u32, *rs1 as u32, *rs2 as u32),
        Inst::Jal { rd, target, symbol } => {
            let offset = match target {
                Operand::Imm(value) => *value,
                Operand::Sym => {
                    let target = symbols
                        .get(symbol)
                        .ok_or_else(|| AsmError::new(0, format!("undefined symbol `{symbol}`")))?;
                    *target as i64 - addr as i64
                }
            };
            if offset % 2 != 0 {
                return Err(AsmError::new(
                    0,
                    format!("jump target is not 2-byte aligned (offset {offset})"),
                ));
            }
            if !fits_signed(offset, 21) {
                return Err(AsmError::new(
                    0,
                    format!("jump offset out of range: {offset}"),
                ));
            }
            isa::jal(*rd as u32, offset as i32)
        }
        Inst::Ebreak => isa::ebreak(),
        Inst::Ecall => isa::ecall(),
    })
}

fn strip_comment(line: &str) -> &str {
    let mut end = line.len();
    for (i, _) in line.char_indices() {
        let rest = &line[i..];
        if rest.starts_with("//") || rest.starts_with(';') || rest.starts_with('#') {
            end = i;
            break;
        }
    }
    &line[..end]
}

fn define_labels<'a>(
    line: &'a str,
    pc: u64,
    symbols: &mut BTreeMap<String, u64>,
    line_no: usize,
) -> Result<&'a str, AsmError> {
    let mut rest = line;
    loop {
        let trimmed = rest.trim_start();
        let end = trimmed
            .find(|c: char| c.is_whitespace())
            .unwrap_or(trimmed.len());
        let token = &trimmed[..end];
        if let Some(name) = token.strip_suffix(':') {
            if name.is_empty() {
                return Err(AsmError::new(line_no, "empty label name"));
            }
            if symbols.contains_key(name) {
                return Err(AsmError::new(line_no, format!("duplicate label `{name}`")));
            }
            symbols.insert(name.to_string(), pc);
            rest = &trimmed[end..];
        } else {
            return Ok(trimmed);
        }
    }
}

fn tokenize(statement: &str) -> Vec<&str> {
    statement
        .split([',', ' ', '\t'])
        .filter(|s| !s.is_empty())
        .collect()
}

fn expect_count(args: &[&str], expected: usize, line_no: usize, who: &str) -> Result<(), AsmError> {
    if args.len() != expected {
        return Err(AsmError::new(
            line_no,
            format!("`{who}` expects {expected} operand(s), got {}", args.len()),
        ));
    }
    Ok(())
}

fn parse_instruction(statement: &str, line_no: usize) -> Result<Inst, AsmError> {
    let args = tokenize(statement);
    let mnemonic = args[0].to_ascii_lowercase();
    let operands = &args[1..];

    let inst = match mnemonic.as_str() {
        "addi" => {
            expect_count(operands, 3, line_no, "addi")?;
            Inst::Addi {
                rd: parse_reg(operands[0], line_no)?,
                rs1: parse_reg(operands[1], line_no)?,
                imm: parse_imm(operands[2], line_no)? as i32,
            }
        }
        "add" | "sub" => {
            expect_count(operands, 3, line_no, &mnemonic)?;
            let rd = parse_reg(operands[0], line_no)?;
            let rs1 = parse_reg(operands[1], line_no)?;
            let rs2 = parse_reg(operands[2], line_no)?;
            if mnemonic == "add" {
                Inst::Add { rd, rs1, rs2 }
            } else {
                Inst::Sub { rd, rs1, rs2 }
            }
        }
        "nop" => {
            expect_count(operands, 0, line_no, "nop")?;
            Inst::Addi {
                rd: 0,
                rs1: 0,
                imm: 0,
            }
        }
        "mov" => {
            expect_count(operands, 2, line_no, "mov")?;
            let rd = parse_reg(operands[0], line_no)?;
            match parse_reg(operands[1], line_no) {
                Ok(rs) => Inst::Addi {
                    rd,
                    rs1: rs,
                    imm: 0,
                },
                Err(_) => {
                    let imm = imm12(parse_imm(operands[1], line_no)?, line_no, "mov")?;
                    Inst::Addi { rd, rs1: 0, imm }
                }
            }
        }
        "li" => {
            expect_count(operands, 2, line_no, "li")?;
            let rd = parse_reg(operands[0], line_no)?;
            let imm = imm12(parse_imm(operands[1], line_no)?, line_no, "li")?;
            Inst::Addi { rd, rs1: 0, imm }
        }
        "jal" | "j" | "call" => {
            if operands.is_empty() || operands.len() > 2 {
                return Err(AsmError::new(
                    line_no,
                    format!("`{mnemonic}` expects one or two operands"),
                ));
            }
            let (rd, target) = match mnemonic.as_str() {
                "j" => {
                    expect_count(operands, 1, line_no, "j")?;
                    (0, operands[0])
                }
                "call" => {
                    expect_count(operands, 1, line_no, "call")?;
                    (1, operands[0])
                }
                _ if operands.len() == 2 => (parse_reg(operands[0], line_no)?, operands[1]),
                _ => (1, operands[0]),
            };
            let (target, symbol) = parse_target(target, line_no)?;
            Inst::Jal { rd, target, symbol }
        }
        "ebreak" => {
            expect_count(operands, 0, line_no, "ebreak")?;
            Inst::Ebreak
        }
        "ecall" => {
            expect_count(operands, 0, line_no, "ecall")?;
            Inst::Ecall
        }
        other => {
            return Err(AsmError::new(
                line_no,
                format!("unknown instruction `{other}`"),
            ))
        }
    };

    Ok(inst)
}

fn parse_target(token: &str, line_no: usize) -> Result<(Operand, String), AsmError> {
    match parse_imm(token, line_no) {
        Ok(value) => Ok((Operand::Imm(value), String::new())),
        Err(_) => Ok((Operand::Sym, token.to_string())),
    }
}

fn imm12(value: i64, line_no: usize, who: &str) -> Result<i32, AsmError> {
    if !fits_signed(value, 12) {
        return Err(AsmError::new(
            line_no,
            format!("`{who}` immediate {value} does not fit 12 bits (lui not implemented yet)"),
        ));
    }
    Ok(value as i32)
}

fn fits_signed(value: i64, bits: u32) -> bool {
    let min = -(1i64 << (bits - 1));
    let max = (1i64 << (bits - 1)) - 1;
    (min..=max).contains(&value)
}

fn parse_reg(token: &str, line_no: usize) -> Result<usize, AsmError> {
    let lower = token.to_ascii_lowercase();
    let named = match lower.as_str() {
        "zero" => Some(0),
        "ra" => Some(1),
        "sp" => Some(2),
        "gp" => Some(3),
        "tp" => Some(4),
        "t0" => Some(5),
        "t1" => Some(6),
        "t2" => Some(7),
        "s0" | "fp" => Some(8),
        "s1" => Some(9),
        "a0" => Some(10),
        "a1" => Some(11),
        "a2" => Some(12),
        "a3" => Some(13),
        "t3" => Some(14),
        "t4" => Some(15),
        _ => None,
    };
    if let Some(index) = named {
        return Ok(index);
    }
    if let Some(number) = lower.strip_prefix('r') {
        if let Ok(index) = number.parse::<usize>() {
            if index < 16 {
                return Ok(index);
            }
            return Err(AsmError::new(
                line_no,
                format!("register `{token}` out of range (r0..r15)"),
            ));
        }
    }
    Err(AsmError::new(
        line_no,
        format!("invalid register `{token}`"),
    ))
}

fn parse_imm(token: &str, line_no: usize) -> Result<i64, AsmError> {
    let bytes = token.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'' {
        let inner = &token[1..token.len() - 1];
        let mut chars = inner.chars();
        let ch = match chars.next() {
            Some('\\') => match chars.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('0') => '\0',
                Some('\\') => '\\',
                Some('\'') => '\'',
                Some('"') => '"',
                Some(other) => other,
                None => return Err(AsmError::new(line_no, "bad escape in char literal")),
            },
            Some(ch) => ch,
            None => return Err(AsmError::new(line_no, "empty char literal")),
        };
        if chars.next().is_some() && !inner.starts_with('\\') {
            return Err(AsmError::new(
                line_no,
                format!("char literal `{token}` must contain one character"),
            ));
        }
        return Ok(ch as i64);
    }

    let (negative, digits) = match token.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, token.strip_prefix('+').unwrap_or(token)),
    };

    let value = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16)
    } else if let Some(bin) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        i64::from_str_radix(bin, 2)
    } else {
        digits.parse::<i64>()
    }
    .map_err(|_| AsmError::new(line_no, format!("invalid number `{token}`")))?;

    Ok(if negative { -value } else { value })
}

fn extract_string(statement: &str, line_no: usize) -> Result<String, AsmError> {
    let start = statement
        .find('"')
        .ok_or_else(|| AsmError::new(line_no, "expected a quoted string"))?;
    let end = statement[start + 1..]
        .find('"')
        .ok_or_else(|| AsmError::new(line_no, "unterminated string"))?
        + start
        + 1;
    let inner = &statement[start + 1..end];
    let mut result = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('0') => result.push('\0'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some(other) => result.push(other),
                None => return Err(AsmError::new(line_no, "bad escape in string")),
            }
        } else {
            result.push(ch);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_cpu::{Cpu, StepOutcome};

    fn run(source: &str) -> Cpu {
        let assembled = assemble(source).unwrap();
        let mut cpu = Cpu::new();
        cpu.load_program(&assembled.bytes, assembled.origin)
            .unwrap();
        for _ in 0..10_000 {
            match cpu.step().unwrap() {
                StepOutcome::Halt => return cpu,
                StepOutcome::Continue => {}
            }
        }
        panic!("program did not halt");
    }

    #[test]
    fn single_addi() {
        let assembled = assemble("addi r1, r0, 42").unwrap();
        assert_eq!(assembled.bytes, isa::addi(1, 0, 42).to_le_bytes());
    }

    #[test]
    fn symbols_are_recorded() {
        let assembled = assemble("start:\n  nop\nloop:\n  nop\n  j loop\n").unwrap();
        assert_eq!(assembled.symbols["start"], 0);
        assert_eq!(assembled.symbols["loop"], 4);
    }

    #[test]
    fn mov_register_and_immediate() {
        let cpu = run("mov r1, r0\nmov r2, 7\nmov a0, t0\nebreak\n");
        assert_eq!(cpu.reg(1), 0);
        assert_eq!(cpu.reg(2), 7);
        assert_eq!(cpu.reg(10), cpu.reg(5));
    }

    #[test]
    fn jump_over_code() {
        let cpu = run("j end\n\
             addi r2, r0, 111\n\
             end:\n\
             addi r3, r0, 222\n\
             ebreak\n");
        assert_eq!(cpu.reg(2), 0);
        assert_eq!(cpu.reg(3), 222);
    }

    #[test]
    fn call_sets_return_register() {
        let cpu = run("call func\n\
             ebreak\n\
             func:\n\
             addi r3, r0, 5\n\
             ebreak\n");
        assert_eq!(cpu.reg(1), 4);
        assert_eq!(cpu.reg(3), 5);
    }

    #[test]
    fn jal_with_explicit_offset() {
        let cpu = run("jal r5, 8\naddi r2, r0, 1\naddi r3, r0, 2\nebreak\n");
        assert_eq!(cpu.reg(5), 4);
        assert_eq!(cpu.reg(2), 0);
        assert_eq!(cpu.reg(3), 2);
    }

    #[test]
    fn data_directives_emit_bytes() {
        let assembled = assemble(".byte 1, 2, 3\n.word 0x11223344\n.asciz \"hi\"\n").unwrap();
        assert_eq!(
            assembled.bytes,
            vec![1, 2, 3, 0x44, 0x33, 0x22, 0x11, b'h', b'i', 0]
        );
    }

    #[test]
    fn org_pads_forward() {
        let assembled = assemble(".org 0x10\naddi r1, r0, 1\n").unwrap();
        assert_eq!(assembled.bytes.len(), 0x14);
        assert!(assembled.bytes[..0x10].iter().all(|b| *b == 0));
    }

    #[test]
    fn comments_are_stripped() {
        let assembled = assemble("addi r1, r0, 1 ; set r1\n# whole line\n").unwrap();
        assert_eq!(assembled.bytes, isa::addi(1, 0, 1).to_le_bytes());
    }

    #[test]
    fn char_literal_immediate() {
        let assembled = assemble("addi r1, r0, 'A'\n").unwrap();
        assert_eq!(assembled.bytes, isa::addi(1, 0, 65).to_le_bytes());
    }

    #[test]
    fn undefined_symbol_errors() {
        let err = assemble("j nowhere\n").unwrap_err();
        assert!(err.message.contains("undefined symbol"));
    }

    #[test]
    fn bad_register_errors() {
        let err = assemble("addi r99, r0, 1\n").unwrap_err();
        assert!(err.message.contains("out of range"));
    }

    #[test]
    fn unknown_mnemonic_errors() {
        let err = assemble("frobnicate r1, r2\n").unwrap_err();
        assert!(err.message.contains("unknown instruction"));
    }

    #[test]
    fn oversized_immediate_errors() {
        let err = assemble("li r1, 100000\n").unwrap_err();
        assert!(err.message.contains("does not fit 12 bits"));
    }

    #[test]
    fn duplicate_label_errors() {
        let err = assemble("a:\nnop\na:\nnop\n").unwrap_err();
        assert!(err.message.contains("duplicate label"));
    }

    #[test]
    fn origin_offsets_addresses() {
        let assembled = assemble_at("j target\ntarget:\nebreak\n", 0x1000).unwrap();
        assert_eq!(assembled.origin, 0x1000);
        assert_eq!(assembled.symbols["target"], 0x1004);
        assert_eq!(assembled.bytes.len(), 8);
    }
}
