// Central loop versus a Duplicated-Match Dispatch

// Made by VBXQ (Haydar)(Celeste) - 2026

// Here's the question: 
// 
// If every opcode handler ends with its own copy of the full
// dispatch match, will LLVM merge/inline them into computed-goto-style
// threaded dispatch? 
// 
// Or does it just bloat code with redundant matches?

// This is my try on optimizing virtual machine/interpreters written in Rust
// Made a really simple VM here just showcase it 

// TLDR;- it works ! 

use std::hint::black_box;
use std::time::Instant;

const OP_HALT: u8 = 0;
const OP_LOADI: u8 = 1;
const OP_ADD: u8 = 2;
const OP_SUB: u8 = 3;
const OP_MUL: u8 = 4;
const OP_DIV: u8 = 5;
const OP_MOD: u8 = 6;
const OP_INC: u8 = 7;
const OP_DEC: u8 = 8;
const OP_JMPNZ: u8 = 9;
const OP_MOV: u8 = 10;

#[inline(always)]
fn encode(op: u8, dst: u8, a: u8, b: u8) -> u32 {
    (op as u32) | ((dst as u32) << 8) | ((a as u32) << 16) | ((b as u32) << 24)
}

#[inline(always)]
fn imm16(a: u8, b: u8) -> i64 {
    ((a as u16) | ((b as u16) << 8)) as i64
}

const NREGS: usize = 16;

// execute one opcode, mutating regs/pc, and returns Some(val) on halt, it's shared by both versions so the actual computation is identiacal
macro_rules! exec_one {
    ($code:expr, $regs:expr, $pc:expr) => {{
        let instr = *unsafe { $code.get_unchecked($pc) };
        let op = (instr & 0xFF) as u8;
        let dst = ((instr >> 8) & 0xFF) as usize;
        let a = ((instr >> 16) & 0xFF) as u8;
        let b = ((instr >> 24) & 0xFF) as u8;
        $pc += 1;
        (op, dst, a, b)
    }};
}

// macro that does the work for one decoded instruction., Some(val) on Halt, and None otherwise
macro_rules! handle {
    ($regs:expr, $pc:expr, $op:expr, $dst:expr, $a:expr, $b:expr) => {
        match $op {
            OP_HALT => return $regs[$dst],
            OP_LOADI => { $regs[$dst] = imm16($a, $b); }
            OP_ADD => { $regs[$dst] = $regs[$a as usize].wrapping_add($regs[$b as usize]); }
            OP_SUB => { $regs[$dst] = $regs[$a as usize].wrapping_sub($regs[$b as usize]); }
            OP_MUL => { $regs[$dst] = $regs[$a as usize].wrapping_mul($regs[$b as usize]); }
            OP_DIV => {
                let d = $regs[$b as usize];
                $regs[$dst] = if d != 0 { $regs[$a as usize] / d } else { 0 };
            }
            OP_MOD => {
                let d = $regs[$b as usize];
                $regs[$dst] = if d != 0 { $regs[$a as usize] % d } else { 0 };
            }
            OP_INC => { $regs[$dst] = $regs[$dst].wrapping_add(1); }
            OP_DEC => { $regs[$dst] = $regs[$dst].wrapping_sub(1); }
            OP_JMPNZ => {
                if $regs[$dst] != 0 { $pc = imm16($a, $b) as usize; }
            }
            OP_MOV => { $regs[$dst] = $regs[$a as usize]; }
            _ => return -1,
        }
    };
}

// here's our test program, it just computes :
//
// sum = 0;
// for i in (0..N) {
//    sum += i*i - i + 1
// }
//

fn make_program(n: u16) -> Vec<u32> {
    let nh = (n & 0xFF) as u8;
    let nl = ((n >> 8) & 0xFF) as u8;
    vec![
        encode(OP_LOADI, 0, nh, nl),  // r0 = N
        encode(OP_LOADI, 1, 0, 0),    // r1 = 0 (le accumulator)
        encode(OP_LOADI, 2, 1, 0),    // r2 = 1
        // loop: (pc = 3)
        encode(OP_MOV, 3, 0, 0),      // r3 = r0
        encode(OP_MUL, 4, 3, 3),      // r4 = r3*r3
        encode(OP_SUB, 5, 4, 3),      // r5 = r4 - r3
        encode(OP_ADD, 5, 5, 2),      // r5 = r5 + 1
        encode(OP_ADD, 1, 1, 5),      // r1 += r5
        encode(OP_DEC, 0, 0, 0),      // r0--
        encode(OP_JMPNZ, 0, 3, 0),   // if r0 != 0 goto 3

        encode(OP_HALT, 1, 0, 0),     // return r1
    ]
}


//////////////////////////////////////////////////////
// VERSION A : Classic dispatch loop
//////////////////////////////////////////////////////
// one decode+math per iteration, all arms jump back to loop head!
#[inline(never)]
fn run_central(code: &[u32]) -> i64 {
    let mut regs = [0i64; NREGS];
    let mut pc: usize = 0;

    loop {
        let (op, dst, a, b) = exec_one!(code, regs, pc);
        handle!(regs, pc, op, dst, a, b);
    }
}

//////////////////////////////////////////////////////
// VERSION B : Duplicated match at tail of every handler
//////////////////////////////////////////////////////
// here's my strategy : each match arms executes the handler, then inline decodes the next instruction, and dispatches it
// through a second inner match, the inner match arms do their work and continue the outer loop

// so, this gives LLVM 11 copieis of the dispatch table, one at the tail of each handler
// if LLVM threads the dispatch, each of those 11 copies becomes an indirect branch! so, computed goto
// if LLVM tail merges them, they collapse into one so same as version A

// the outer loop here is only needed as a "safety net", in a fully threaded execution the contiinue at the bottom
// of the inner match keeps bouncing through outer => handler => inner dispatch => handler and so on
#[inline(never)]
fn run_threaded(code: &[u32]) -> i64 {
    let mut regs = [0i64; NREGS];
    let mut pc: usize = 0;

    loop {
        let (op, dst, a, b) = exec_one!(code, regs, pc);
        match op {
            OP_HALT => return regs[dst],
            OP_LOADI => {
                regs[dst] = imm16(a, b);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_ADD => {
                regs[dst] = regs[a as usize].wrapping_add(regs[b as usize]);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_SUB => {
                regs[dst] = regs[a as usize].wrapping_sub(regs[b as usize]);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_MUL => {
                regs[dst] = regs[a as usize].wrapping_mul(regs[b as usize]);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_DIV => {
                let d = regs[b as usize];
                regs[dst] = if d != 0 { regs[a as usize] / d } else { 0 };
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_MOD => {
                let d = regs[b as usize];
                regs[dst] = if d != 0 { regs[a as usize] % d } else { 0 };
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_INC => {
                regs[dst] = regs[dst].wrapping_add(1);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_DEC => {
                regs[dst] = regs[dst].wrapping_sub(1);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_JMPNZ => {
                if regs[dst] != 0 { pc = imm16(a, b) as usize; }
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            OP_MOV => {
                regs[dst] = regs[a as usize];
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle!(regs, pc, op2, dst2, a2, b2);
            }
            _ => return -1,
        }
    }
}

//////////////////////////////////////////////////////
// VERSION C : deeper unrolling, 3 levels of inline dispatch
//////////////////////////////////////////////////////
// if 2 level isn't enough for LLVM to see the pattern, we can try 3 levels
macro_rules! handle_and_dispatch {
    ($code:expr, $regs:expr, $pc:expr, $op:expr, $dst:expr, $a:expr, $b:expr) => {
        match $op {
            OP_HALT => return $regs[$dst],
            OP_LOADI => { $regs[$dst] = imm16($a, $b); }
            OP_ADD => { $regs[$dst] = $regs[$a as usize].wrapping_add($regs[$b as usize]); }
            OP_SUB => { $regs[$dst] = $regs[$a as usize].wrapping_sub($regs[$b as usize]); }
            OP_MUL => { $regs[$dst] = $regs[$a as usize].wrapping_mul($regs[$b as usize]); }
            OP_DIV => {
                let d = $regs[$b as usize];
                $regs[$dst] = if d != 0 { $regs[$a as usize] / d } else { 0 };
            }
            OP_MOD => {
                let d = $regs[$b as usize];
                $regs[$dst] = if d != 0 { $regs[$a as usize] % d } else { 0 };
            }
            OP_INC => { $regs[$dst] = $regs[$dst].wrapping_add(1); }
            OP_DEC => { $regs[$dst] = $regs[$dst].wrapping_sub(1); }
            OP_JMPNZ => {
                if $regs[$dst] != 0 { $pc = imm16($a, $b) as usize; }
            }
            OP_MOV => { $regs[$dst] = $regs[$a as usize]; }
            _ => return -1,
        }
        // level 3: decode + handle next instruction, then fall through to loop
        let (op3, dst3, a3, b3) = exec_one!($code, $regs, $pc);
        handle!($regs, $pc, op3, dst3, a3, b3);
    };
}

#[inline(never)]
fn run_threaded_deep(code: &[u32]) -> i64 {
    let mut regs = [0i64; NREGS];
    let mut pc: usize = 0;

    loop {
        // level 1: decode + dispatch
        let (op1, dst1, a1, b1) = exec_one!(code, regs, pc);
        match op1 {
            OP_HALT => return regs[dst1],
            OP_LOADI => {
                regs[dst1] = imm16(a1, b1);
                // level 2: full inline dispatch
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_ADD => {
                regs[dst1] = regs[a1 as usize].wrapping_add(regs[b1 as usize]);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_SUB => {
                regs[dst1] = regs[a1 as usize].wrapping_sub(regs[b1 as usize]);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_MUL => {
                regs[dst1] = regs[a1 as usize].wrapping_mul(regs[b1 as usize]);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_DIV => {
                let d = regs[b1 as usize];
                regs[dst1] = if d != 0 { regs[a1 as usize] / d } else { 0 };
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_MOD => {
                let d = regs[b1 as usize];
                regs[dst1] = if d != 0 { regs[a1 as usize] % d } else { 0 };
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_INC => {
                regs[dst1] = regs[dst1].wrapping_add(1);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_DEC => {
                regs[dst1] = regs[dst1].wrapping_sub(1);
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_JMPNZ => {
                if regs[dst1] != 0 { pc = imm16(a1, b1) as usize; }
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            OP_MOV => {
                regs[dst1] = regs[a1 as usize];
                let (op2, dst2, a2, b2) = exec_one!(code, regs, pc);
                handle_and_dispatch!(code, regs, pc, op2, dst2, a2, b2);
            }
            _ => return -1,
        }
    }
}

// le benchmark
fn bench<F: Fn(&[u32]) -> i64>(name: &str, code: &[u32], iters: u32, f: F) {
    for _ in 0..100 {
        black_box(f(black_box(code)));
    }

    let start = Instant::now();
    for _ in 0..iters {
        black_box(f(black_box(code)));
    }
    let elapsed = start.elapsed();

    let result = f(code);
    let ns_per_iter = elapsed.as_nanos() as f64 / iters as f64;
    println!("{name:>24}: {ns_per_iter:8.1} ns/iter  (result = {result})");
}

fn main() {
    let program = make_program(1000);
    let iters = 100_000;

    println!("VM Dispatch Benchmark");
    println!("Program: sum(i*i - i + 1) for i in 1..=1000");
    println!("Iterations: {iters}\n");

    bench("central-dispatch", &program, iters, run_central);
    bench("threaded-2level", &program, iters, run_threaded);
    bench("threaded-3level", &program, iters, run_threaded_deep);

    println!();
    println!("To inspect assembly:");
    println!("  cargo rustc --release -- --emit=asm");
    println!("  Look in target/release/deps/vm_dispatch_bench-*.s");
    println!();
    println!("To disable tail-merging (force LLVM to keep duplicated dispatch):");
    println!("  set RUSTFLAGS=-C llvm-args=-tail-merge-threshold=0");
    println!("  cargo rustc --release -- --emit=asm");
}
