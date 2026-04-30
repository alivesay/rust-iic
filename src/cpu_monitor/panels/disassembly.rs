use crate::cpu_monitor::panels::breakpoints::BreakpointSet;
use crate::cpu_monitor::panels::iou::softswitch_effect;
use crate::cpu_monitor::state::CpuState;
use crate::disassembler::{lookup_system_symbol, Disassembler};

// How many instructions to disassemble forward from the start address.
const FORWARD_COUNT: usize = 48;
// Maximum bytes to scan backwards from PC trying to find an instruction
// alignment that decodes cleanly into PC. 6502 instructions are 1–3 bytes,
// so a generous sweep gives ~12+ candidate instructions of lead-in.
const MAX_BACKSCAN: u16 = 36;
// Preferred number of instructions to show above PC when auto-following.
// Tuned so PC sits roughly centered in a typical viewport.
const PREFERRED_LEAD_IN: usize = 12;
// Bytes covered by the cached disassembly. 32 instructions max out at
// ~96 bytes; round up so we catch incidental nearby writes.
const CACHE_REGION_BYTES: usize = 128;

#[derive(Clone)]
struct DisasmRow {
    addr: u16,
    // Pre-formatted "AAAA  BB BB BB  MNEMONIC ..." line.
    text: String,
}

pub struct DisassemblyPanelState {
    // User-overridden start address. When `None`, the view follows PC.
    pub start: Option<u16>,
    pub goto: String,
    pub follow_pc: bool,

    // Cached disassembly. Invalidated when the start address changes or
    // when the underlying memory bytes change. PC marker uses live data.
    cache_start: u16,
    cache_hash: u64,
    cache_rows: Vec<DisasmRow>,

    // Diagnostics: cache stats since last reset.
    pub cache_hits: u64,
    pub cache_misses: u64,
}

impl Default for DisassemblyPanelState {
    fn default() -> Self {
        Self {
            start: None,
            goto: String::new(),
            follow_pc: true,
            cache_start: 0,
            cache_hash: 0,
            cache_rows: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
        }
    }
}

// Pick a start address such that decoding forward from it lands exactly on
// Find the first 8- or 16-bit hex literal in a disassembled instruction
// and return the matching annotation (system symbol, or — for $C0xx —
// the soft-switch effect, which is far more useful than a label like
// `TXTCLR` when reading code). Skips immediate operands (e.g.
// `LDA #$20`).
fn first_symbol_for(text: &str) -> Option<&'static str> {
    // Determine whether this instruction stores to its operand. STA/STX/STY/STZ
    // are the only common 6502 writes; everything else reads (or both, like
    // INC/DEC/RMW which still trigger softswitches identically). Used by
    // `softswitch_effect` to disambiguate the rare addresses where R vs W
    // differ.
    let is_write = matches!(
        text.as_bytes().first().copied(),
        Some(b'S')
    ) && {
        let head = text.as_bytes();
        head.len() >= 3
            && head[0] == b'S'
            && matches!(head[1], b'T')
            && matches!(head[2], b'A' | b'X' | b'Y' | b'Z')
    };

    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            // Skip immediate `#$xx` operands.
            if i > 0 && bytes[i - 1] == b'#' {
                i += 1;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end] as char).is_ascii_hexdigit()
                && end - start < 4
            {
                end += 1;
            }
            let hex = &text[start..end];
            if !hex.is_empty() {
                if let Ok(addr) = u16::from_str_radix(hex, 16) {
                    // Zero-page references resolve into 0x00xx.
                    let lookup = if hex.len() <= 2 { addr & 0x00FF } else { addr };
                    // For $C0xx the soft-switch effect is much more
                    // informative than the bare system symbol; prefer it.
                    if (0xC000..=0xC0FF).contains(&lookup) {
                        if let Some(eff) = softswitch_effect(lookup, is_write) {
                            return Some(eff);
                        }
                    }
                    if let Some(sym) = lookup_system_symbol(lookup) {
                        return Some(sym);
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

// Pick a start address such that decoding forward from it lands exactly on
// `pc`, giving roughly `PREFERRED_LEAD_IN` instructions of context above.
//
// 6502 has variable-length (1–3 byte) instructions and no alignment, so the
// only way to disassemble correctly *before* a known PC is to try several
// candidate starts and pick one whose decoded stream actually hits PC.
fn find_aligned_start(pc: u16, memory_reader: &dyn Fn(u16) -> u8) -> u16 {
    // Walk back one byte at a time. For each candidate start, simulate the
    // decode and count how many instructions land before PC. Track the
    // candidate that produces the most instructions of lead-in without
    // exceeding the preferred count, and which actually hits PC exactly.
    let mut best: Option<(u16, usize)> = None;
    for back in 1..=MAX_BACKSCAN {
        let start = pc.wrapping_sub(back);
        let mut addr = start;
        let mut lead = 0usize;
        let mut hit = false;
        for _ in 0..MAX_BACKSCAN as usize + 2 {
            if addr == pc {
                hit = true;
                break;
            }
            if (pc.wrapping_sub(addr)) > MAX_BACKSCAN {
                break;
            }
            let (_, len) = Disassembler::disassemble_peek(addr, memory_reader);
            let len = len.max(1) as u16;
            addr = addr.wrapping_add(len);
            lead += 1;
            if lead > PREFERRED_LEAD_IN + 1 {
                break;
            }
        }
        if hit {
            // Prefer the alignment closest to PREFERRED_LEAD_IN.
            let score = lead;
            let better = match best {
                None => true,
                Some((_, b_lead)) => {
                    let d_new = score.abs_diff(PREFERRED_LEAD_IN);
                    let d_old = b_lead.abs_diff(PREFERRED_LEAD_IN);
                    d_new < d_old
                }
            };
            if better {
                best = Some((start, score));
            }
        }
    }
    best.map(|(s, _)| s).unwrap_or(pc)
}

// Cheap FNV-1a-style 64-bit checksum over the disasm window. Good enough
// to detect any change; not cryptographic.
fn region_hash(start: u16, memory_reader: &dyn Fn(u16) -> u8) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for i in 0..CACHE_REGION_BYTES as u16 {
        let b = memory_reader(start.wrapping_add(i));
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn rebuild_cache(
    state: &mut DisassemblyPanelState,
    start_addr: u16,
    memory_reader: &dyn Fn(u16) -> u8,
) {
    state.cache_rows.clear();
    let mut addr = start_addr;
    for _ in 0..FORWARD_COUNT {
        let (text, len) = Disassembler::disassemble_peek(addr, memory_reader);
        let len = len.max(1);

        let mut line = String::with_capacity(48);
        use std::fmt::Write;
        let _ = write!(line, "{:04X}  ", addr);
        for i in 0..len {
            let _ = write!(line, "{:02X} ", memory_reader(addr.wrapping_add(i as u16)));
        }
        // Pad bytes column to 9 chars (3 bytes * 3 + extra spaces)
        while line.len() < 4 + 2 + 9 {
            line.push(' ');
        }
        line.push(' ');
        line.push_str(&text);

        // annotate
        if let Some(sym) = first_symbol_for(&text) {
            const SYMBOL_COL: usize = 32;
            while line.len() < SYMBOL_COL {
                line.push(' ');
            }
            let _ = write!(line, "  ; {}", sym);
        }

        state.cache_rows.push(DisasmRow { addr, text: line });

        let next = addr.wrapping_add(len as u16);
        if next < start_addr && start_addr.wrapping_add(64) < next {
            break;
        }
        addr = next;
    }
}

pub fn render_toolbar(ui: &mut egui::Ui, state: &mut DisassemblyPanelState) {
    ui.checkbox(&mut state.follow_pc, "Follow PC");
    ui.separator();
    ui.label("goto");
    let resp = ui.add(
        egui::TextEdit::singleline(&mut state.goto)
            .desired_width(60.0)
            .hint_text("hex"),
    );
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Ok(addr) = u16::from_str_radix(state.goto.trim(), 16) {
            state.start = Some(addr);
            state.follow_pc = false;
        }
        state.goto.clear();
    }
    if ui
        .small_button("\u{21BA} PC")
        .on_hover_text("Snap back to PC")
        .clicked()
    {
        state.start = None;
        state.follow_pc = true;
    }
}

pub fn render(
    ui: &mut egui::Ui,
    state: &mut DisassemblyPanelState,
    cpu_state: &CpuState,
    breakpoints: &mut BreakpointSet,
    memory_reader: &dyn Fn(u16) -> u8,
) {

    let start_addr = if state.follow_pc {
        find_aligned_start(cpu_state.pc, memory_reader)
    } else {
        state.start.unwrap_or(cpu_state.pc)
    };

    let needs_rebuild = state.cache_rows.is_empty() || state.cache_start != start_addr || {
        let h = region_hash(start_addr, memory_reader);
        if h != state.cache_hash {
            state.cache_hash = h;
            true
        } else {
            false
        }
    };
    if needs_rebuild {
        state.cache_start = start_addr;
        state.cache_hash = region_hash(start_addr, memory_reader);
        rebuild_cache(state, start_addr, memory_reader);
        state.cache_misses += 1;
    } else {
        state.cache_hits += 1;
    }

    let pc_color = egui::Color32::from_rgb(0xE5, 0xC0, 0x70);
    let dim = ui.visuals().weak_text_color();
    let pc_bg = egui::Color32::from_rgba_unmultiplied(0xE5, 0xC0, 0x70, 0x22);
    let bp_color = egui::Color32::from_rgb(0xC8, 0x55, 0x40);
    let bp_bg = egui::Color32::from_rgba_unmultiplied(0xC8, 0x55, 0x40, 0x22);

    egui::ScrollArea::vertical()
        .id_salt("cpu_disasm")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in &state.cache_rows {
                let is_pc = row.addr == cpu_state.pc;
                let has_bp = breakpoints.contains(row.addr);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    let gutter_text = if has_bp { "\u{25CF}" } else { "\u{00B7}" };
                    let gutter_color = if has_bp { bp_color } else { dim };
                    let gutter = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(gutter_text)
                                    .monospace()
                                    .color(gutter_color),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text(if has_bp {
                            "Click to remove breakpoint"
                        } else {
                            "Click to set breakpoint"
                        });
                    if gutter.clicked() {
                        breakpoints.toggle(row.addr);
                    }

                    let prefix = if is_pc { "\u{25B6} " } else { "  " };
                    let line = format!("{}{}", prefix, row.text);
                    let mut rich = egui::RichText::new(line).monospace();
                    if is_pc {
                        rich = rich.strong().color(pc_color).background_color(pc_bg);
                    } else if has_bp {
                        rich = rich.color(bp_color).background_color(bp_bg);
                    } else {
                        rich = rich.color(dim);
                    }

                    let resp = ui.add(
                        egui::Label::new(rich).sense(egui::Sense::click()),
                    );
                    if resp.clicked() {
                        breakpoints.toggle(row.addr);
                    }
                    if is_pc && state.follow_pc {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                });
            }
        });
}
