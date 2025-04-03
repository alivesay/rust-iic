use crate::cpu::CPU;
use crate::rom::ROM;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

pub struct Monitor<'a> {
    cpu: &'a mut CPU,
    breakpoints: HashSet<u16>,
}

impl<'a> Monitor<'a> {
    pub fn new(cpu: &'a mut CPU) -> Self {
        cpu.bus.interrupts.enter_halt();
        Self {
            cpu,
            breakpoints: HashSet::new(),
        }
    }

    pub fn repl(&mut self) {
        self.load_rc_file();

        let stdin = io::stdin();
        loop {
            print!("monitor> ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            stdin.read_line(&mut input).unwrap();
            let input = input.trim();

            if input.is_empty() {
                continue;
            }

            self.execute_command(input);
        }
    }

    fn execute_command(&mut self, input: &str) {
        let args: Vec<&str> = input.split_whitespace().collect();

        if args.is_empty() {
            return;
        }

        match args[0] {
            "help" => self.show_help(),
            "reset" => self.cpu.reset(),
            "step" | "s" => self.step(),
            "continue" | "c" => self.resume(),
            "break" if args.len() == 2 => self.set_breakpoint(args[1]),
            "delete" if args.len() == 2 => self.remove_breakpoint(args[1]),
            "registers" | "r" => self.show_registers(),
            "flags" => self.show_flags(),
            "halt" => self.halt_cpu(),
            "load" if args.len() == 2 => self.load_rom(args[1], None),
            "load" if args.len() == 3 => self.load_rom(args[1], Some(args[2])),
            "mem" if args.len() == 2 => self.view_memory(args[1], None),
            "mem" if args.len() == 3 => self.view_memory(args[1], Some(args[2])),
            "page" if args.len() == 2 => self.view_memory_page(args[1]),
            "write" if args.len() == 3 => self.write_memory(args[1], args[2]),
            "exit" | "quit" => {
                println!("Exiting monitor. CPU remains halted.");
                std::process::exit(0);
            }
            _ => println!("Unknown command. Type 'help' for available commands."),
        }
    }

    fn show_help(&self) {
        println!("Available commands:");
        println!("  help           - Show this help message");
        println!("  load <file> [addr]  - Load a ROM file into memory at [addr] (default 0x0000)");
        println!("  reset          - Reset the CPU");
        println!("  step (s)       - Execute a single instruction");
        println!("  continue (c)   - Resume execution from halt/breakpoint");
        println!("  break <addr>   - Set a breakpoint at <addr> (hex)");
        println!("  delete <addr>  - Remove a breakpoint at <addr> (hex)");
        println!("  registers (r)  - Show CPU registers");
        println!("  flags          - Show CPU status flags");
        println!("  halt           - Halt the CPU");
        println!("  mem <addr>     - View memory at <addr> (hex)");
        println!("  mem <start> <end> - View memory range (hex)");
        println!("  page <addr>    - View a full 256-byte memory page");
        println!("  write <addr> <value> - Write <value> (hex) to <addr> (hex)");
        println!("  quit | exit    - Exit the monitor (CPU remains halted)");
    }

    fn load_rc_file(&mut self) {
        let path = Path::new(".monitorrc");
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            for line in reader.lines() {
                match line {
                    Ok(cmd) if !cmd.trim().is_empty() && !cmd.trim().starts_with('#') => {
                        self.execute_command(cmd.trim());
                    }
                    Err(e) => eprintln!("Error reading line: {}", e),
                    _ => {}
                }
            }
        }
    }

    fn load_rom(&mut self, filename: &str, addr: Option<&str>) {
        let load_address = addr
            .and_then(|s| u16::from_str_radix(s, 16).ok())
            .unwrap_or(0x0000);

        match ROM::load_from_file(filename, self.cpu.system_type) {
            Ok(rom) => {
                self.cpu.bus.write_bytes(load_address, &rom.data);
                println!(
                    "Loaded ROM '{}' at ${:04X} ({} bytes)",
                    filename,
                    load_address,
                    rom.data.len()
                );
            }
            Err(err) => println!("Error loading ROM: {}", err),
        }
    }

    fn step(&mut self) {
        if self.cpu.bus.interrupts.halted {
            println!("CPU is halted. Use 'continue' to resume execution.");
            return;
        }

        self.cpu.tick();
        self.show_registers();

        if self.breakpoints.contains(&self.cpu.pc) {
            println!("Hit breakpoint at {:04X}. Execution halted.", self.cpu.pc);
            self.cpu.bus.interrupts.enter_halt();
        }
    }

    fn run(&mut self) {
        while !self.cpu.bus.interrupts.halted {
            if self.breakpoints.contains(&self.cpu.pc) {
                println!("Hit breakpoint at {:04X}. Execution halted.", self.cpu.pc);
                self.cpu.bus.interrupts.enter_halt();
                break;
            }
            self.cpu.tick();
        }
    }

    // TODO: rework halt/wait in this context
    fn resume(&mut self) {
        if !self.cpu.bus.interrupts.halted {
            println!("CPU is already running.");
            return;
        }

        println!("Resuming execution...");
        self.cpu.bus.interrupts.leave_halt();
        self.cpu.bus.interrupts.leave_wait();
        self.run();
    }

    fn halt_cpu(&mut self) {
        self.cpu.bus.interrupts.enter_halt();
        println!("CPU halted.");
    }

    fn set_breakpoint(&mut self, addr: &str) {
        if let Ok(addr) = u16::from_str_radix(addr, 16) {
            self.breakpoints.insert(addr);
            println!("Breakpoint set at ${:04X}", addr);
        }
    }

    fn remove_breakpoint(&mut self, addr: &str) {
        if let Ok(addr) = u16::from_str_radix(addr, 16) {
            self.breakpoints.remove(&addr);
            println!("Breakpoint removed at ${:04X}", addr);
        }
    }

    fn show_registers(&self) {
        println!(
            "PC: {:04X}  A: {:02X}  X: {:02X}  Y: {:02X}  SP: {:02X}",
            self.cpu.pc, self.cpu.regs.a, self.cpu.regs.x, self.cpu.regs.y, self.cpu.regs.sp
        );
    }

    fn show_flags(&self) {
        println!(
            "Flags: C={} Z={} I={} D={} B={} V={} N={}",
            self.cpu.p.contains(crate::cpu::Flags::CARRY) as u8,
            self.cpu.p.contains(crate::cpu::Flags::ZERO) as u8,
            self.cpu.p.contains(crate::cpu::Flags::IRQ_DISABLE) as u8,
            self.cpu.p.contains(crate::cpu::Flags::DECIMAL) as u8,
            self.cpu.p.contains(crate::cpu::Flags::BREAK) as u8,
            self.cpu.p.contains(crate::cpu::Flags::OVERFLOW) as u8,
            self.cpu.p.contains(crate::cpu::Flags::NEGATIVE) as u8
        );
    }

    fn view_memory(&self, start: &str, end: Option<&str>) {
        if let Ok(start_addr) = u16::from_str_radix(start, 16) {
            let end_addr = end
                .and_then(|e| u16::from_str_radix(e, 16).ok())
                .unwrap_or(start_addr);

            if start_addr > end_addr {
                println!("Invalid range: start address must be <= end address");
                return;
            }

            for addr in start_addr..=end_addr {
                let value = self.cpu.bus.read_byte(addr);
                println!("${:04X}: {:02X}", addr, value);
            }
        }
    }

    fn view_memory_page(&self, addr: &str) {
        if let Ok(addr) = u16::from_str_radix(addr, 16) {
            let page_start = addr & 0xFF00; // align to $XX00
            for i in 0..16 {
                let offset = i * 16;
                print!("${:04X}: ", page_start + offset);
                for j in 0..16 {
                    let value = self.cpu.bus.read_byte(page_start + offset + j);
                    print!("{:02X} ", value);
                }
                println!();
            }
        }
    }

    fn write_memory(&mut self, addr: &str, value: &str) {
        if let (Ok(addr), Ok(value)) =
            (u16::from_str_radix(addr, 16), u8::from_str_radix(value, 16))
        {
            self.cpu.bus.write_byte(addr, value);
            println!("Wrote {:02X} to ${:04X}", value, addr);
        }
    }
}
