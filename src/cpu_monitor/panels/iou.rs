use crate::cpu_monitor::state::IouSnapshot;
use crate::cpu_monitor::widgets::flag_chip;
use crate::mmu::MemStateMask;
use crate::timing;
use crate::video::VideoModeMask;

pub fn iou_body(ui: &mut egui::Ui, iou: &IouSnapshot) {
    let dim = ui.visuals().weak_text_color();
    ui.horizontal(|ui| {
        flag_chip(ui, "IOUDIS",  iou.ioudis);
        flag_chip(ui, "80COL",   iou.col80_switch);
        flag_chip(ui, "3.5DSK",  iou.disk35_mode);
        flag_chip(ui, "STEST",   iou.self_test);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("FB ${:02X}", iou.floating_bus))
                    .monospace()
                    .strong()
                    .color(dim),
            )
            .on_hover_text(
                "Floating bus: byte the video circuitry most recently \
                 fetched. Read by software for VBL detection / copy \
                 protection.",
            );
        });
    });

    ui.add_space(6.0);
    section_header(ui, "Input");
    input_summary(ui, iou);

    ui.add_space(6.0);
    section_header(ui, "SCC");
    scc_summary(ui, iou);

    ui.add_space(6.0);

    let tab_id = egui::Id::new("iou_softswitch_tab");
    let mut show_log = ui.data(|d| d.get_temp::<bool>(tab_id).unwrap_or(false));
    ui.horizontal(|ui| {
        if ui.selectable_label(!show_log, "Softswitches").clicked() {
            show_log = false;
        }
        if ui.selectable_label(show_log, "Access log").clicked() {
            show_log = true;
        }
    });
    ui.data_mut(|d| d.insert_temp(tab_id, show_log));
    ui.separator();
    if show_log {
        access_log(ui, iou);
    } else {
        softswitch_list(ui, iou);
    }
}

fn access_log(ui: &mut egui::Ui, iou: &IouSnapshot) {
    let count = iou.recent_access_count as usize;
    if count == 0 {
        ui.label(
            egui::RichText::new("(no $C0xx accesses yet)")
                .monospace().small().color(ui.visuals().weak_text_color()),
        );
        return;
    }

    let dim = ui.visuals().weak_text_color();
    let read_col = egui::Color32::from_rgb(0x70, 0xC0, 0xE0);
    let write_col = egui::Color32::from_rgb(0xE5, 0x90, 0x60);

    egui::ScrollArea::vertical()
        .id_salt("iou_access_log")
        .max_height(140.0)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            egui::Grid::new("iou_access_log_grid")
                .num_columns(6)
                .spacing([8.0, 1.0])
                .min_col_width(0.0)
                .show(ui, |ui| {
                    for entry in &iou.recent_accesses[..count] {
                        let (kind_txt, kind_col) = if entry.write {
                            ("W", write_col)
                        } else {
                            ("R", read_col)
                        };
                        ui.label(
                            egui::RichText::new(kind_txt)
                                .monospace().small().color(kind_col).strong(),
                        );
                        ui.label(
                            egui::RichText::new(format!("${:04X}", entry.addr))
                                .monospace().small(),
                        );
                        ui.label(
                            egui::RichText::new(format!("={:02X}", entry.value))
                                .monospace().small().color(dim),
                        );
                        let effect = softswitch_effect(entry.addr, entry.write).unwrap_or("");
                        ui.label(
                            egui::RichText::new(effect)
                                .monospace().small().color(dim),
                        );
                        ui.label(
                            egui::RichText::new(format!("PC=${:04X}", entry.pc))
                                .monospace().small().color(dim),
                        );
                        ui.label(
                            egui::RichText::new(format!("cyc={}", entry.cycle))
                                .monospace().small().color(dim),
                        );
                        ui.end_row();
                    }
                });
        });
}

pub fn softswitch_effect(addr: u16, _is_write: bool) -> Option<&'static str> {
    Some(match addr {
        // 80STORE / aux-mem switches
        0xC000 => "80STORE off",
        0xC001 => "80STORE on",
        0xC002 => "RAMRD main",
        0xC003 => "RAMRD aux",
        0xC004 => "RAMWRT main",
        0xC005 => "RAMWRT aux",
        0xC006 => "INTCXROM off",
        0xC007 => "INTCXROM on",
        0xC008 => "ALTZP main",
        0xC009 => "ALTZP aux",
        0xC00A => "SLOTC3ROM off",
        0xC00B => "SLOTC3ROM on",
        0xC00C => "80COL off",
        0xC00D => "80COL on",
        0xC00E => "ALTCHAR off",
        0xC00F => "ALTCHAR on",

        // Status reads
        0xC010 => "kbd strobe clear",
        0xC011 => "RD LCBNK2",
        0xC012 => "RD LCRAM",
        0xC013 => "RD RAMRD",
        0xC014 => "RD RAMWRT",
        0xC015 => "RD XINT",
        0xC016 => "RD ALTZP",
        0xC017 => "RD YINT",
        0xC018 => "RD 80STORE",
        0xC019 => "RD VBL",
        0xC01A => "RD TEXT",
        0xC01B => "RD MIXED",
        0xC01C => "RD PAGE2",
        0xC01D => "RD HIRES",
        0xC01E => "RD ALTCHAR",
        0xC01F => "RD 80COL",

        0xC020 => "tape out toggle",
        0xC030 => "speaker click",
        0xC031 => "DISKREG",

        // Video softswitches
        0xC050 => "TEXT off",
        0xC051 => "TEXT on",
        0xC052 => "MIXED off",
        0xC053 => "MIXED on",
        0xC054 => "PAGE2 off",
        0xC055 => "PAGE2 on",
        0xC056 => "HIRES off",
        0xC057 => "HIRES on",

        // Annunciators (no IOU effect on IIc but still on the bus)
        0xC058 => "AN0 off",
        0xC059 => "AN0 on",
        0xC05A => "AN1 off",
        0xC05B => "AN1 on",
        0xC05C => "AN2 off",
        0xC05D => "AN2 on",
        0xC05E => "AN3 off / dhires on",
        0xC05F => "AN3 on  / dhires off",

        // Mouse / paddle
        0xC040 => "mouse XY mask",
        0xC041 => "mouse VBL mask",
        0xC042 => "mouse X0 edge",
        0xC043 => "mouse Y0 edge",
        0xC060 => "BUTN3 / kbd switch",
        0xC061 => "open-apple btn",
        0xC062 => "closed-apple btn",
        0xC063 => "mouse btn",
        0xC064 => "PADDL0",
        0xC065 => "PADDL1",
        0xC066 => "mouse X1",
        0xC067 => "mouse Y1",
        0xC070 => "PTRIG / mouse VBL clr",

        // Language card bank/mode (LC) — $C080-$C08F.
        // Bit 0 selects bank2(0)/bank1(1); bits 1-2 pick read/write/mode.
        0xC080..=0xC08F => match addr & 0x0F {
            0x00 => "LC rd bnk2 wprot",
            0x01 => "LC rd bnk2 wen",
            0x02 => "LC rom rd bnk2 wprot",
            0x03 => "LC rd bnk2 wen (rd-rom)",
            0x04 => "LC rd bnk2 wprot",
            0x05 => "LC rd bnk2 wen",
            0x06 => "LC rom rd bnk2 wprot",
            0x07 => "LC rd bnk2 wen (rd-rom)",
            0x08 => "LC rd bnk1 wprot",
            0x09 => "LC rd bnk1 wen",
            0x0A => "LC rom rd bnk1 wprot",
            0x0B => "LC rd bnk1 wen (rd-rom)",
            0x0C => "LC rd bnk1 wprot",
            0x0D => "LC rd bnk1 wen",
            0x0E => "LC rom rd bnk1 wprot",
            _    => "LC rd bnk1 wen (rd-rom)",
        },

        // IWM ($C0E0-$C0EF) — bit pattern selects phase/motor/drive/Q6/Q7
        0xC0E0..=0xC0EF => match addr & 0x0F {
            0x00 => "IWM PH0 off",
            0x01 => "IWM PH0 on",
            0x02 => "IWM PH1 off",
            0x03 => "IWM PH1 on",
            0x04 => "IWM PH2 off",
            0x05 => "IWM PH2 on",
            0x06 => "IWM PH3 off",
            0x07 => "IWM PH3 on",
            0x08 => "IWM motor off",
            0x09 => "IWM motor on",
            0x0A => "IWM drive 1",
            0x0B => "IWM drive 2",
            0x0C => "IWM Q6L (read)",
            0x0D => "IWM Q6H",
            0x0E => "IWM Q7L (rd mode)",
            _    => "IWM Q7H (wr mode)",
        },

        _ => return None,
    })
}

fn softswitch_list(ui: &mut egui::Ui, iou: &IouSnapshot) {
    const ENTRIES: &[(u16, &str)] = &[
        (0xC011, "RDLCBNK2"),
        (0xC012, "RDLCRAM"),
        (0xC013, "RDRAMRD"),
        (0xC014, "RDRAMWRT"),
        (0xC015, "RDXINT"),
        (0xC016, "RDALTZP"),
        (0xC017, "RDYINT"),
        (0xC018, "RD80STORE"),
        (0xC01A, "RDTEXT"),
        (0xC01B, "RDMIXED"),
        (0xC01C, "RDPAGE2"),
        (0xC01D, "RDHIRES"),
        (0xC01E, "RDALTCHAR"),
        (0xC01F, "RD80COL"),
        (0xC031, "DISKREG"),
        (0xC040, "RDXYMSK"),
        (0xC041, "RDVBLMSK"),
        (0xC042, "RDX0EDGE"),
        (0xC043, "RDY0EDGE"),
        (0xC060, "BUTN3"),
        (0xC061, "RDBTN0"),
        (0xC062, "RDBTN1"),
        (0xC063, "RDMBTN"),
        (0xC066, "RDMOUX1"),
        (0xC067, "RDMOUY1"),
    ];

    let dim = ui.visuals().weak_text_color();
    let hot = egui::Color32::from_rgb(0xE5, 0xC0, 0x70);
    let half = (ENTRIES.len() + 1) / 2;

    let render_cell = |ui: &mut egui::Ui, addr: u16, mnemonic: &str| {
        let val = iou.softswitches[(addr - 0xC000) as usize];
        ui.label(
            egui::RichText::new(format!("${:04X}", addr))
                .monospace().small().color(dim),
        );
        let (val_txt, val_col) = match val {
            Some(0x00) => ("00".to_string(), dim),
            Some(v)    => (format!("{:02X}", v), hot),
            None       => ("..".to_string(), dim),
        };
        let mut val_rt = egui::RichText::new(val_txt).monospace().small().color(val_col);
        if matches!(val, Some(v) if v != 0) { val_rt = val_rt.strong(); }
        ui.label(val_rt);
        ui.label(egui::RichText::new(mnemonic).monospace().small());
    };

    egui::Grid::new("iou_softswitch_list")
        .num_columns(7)
        .spacing([8.0, 1.0])
        .min_col_width(0.0)
        .show(ui, |ui| {
            for i in 0..half {
                let (addr_l, mnem_l) = ENTRIES[i];
                render_cell(ui, addr_l, mnem_l);

                // gap column between the two halves
                ui.label(egui::RichText::new("  ").monospace().small());

                if let Some(&(addr_r, mnem_r)) = ENTRIES.get(half + i) {
                    render_cell(ui, addr_r, mnem_r);
                } else {
                    ui.label(""); ui.label(""); ui.label("");
                }
                ui.end_row();
            }
        });
}

pub fn render_video_pane(ui: &mut egui::Ui, iou: &IouSnapshot) {
    video_chips(ui, iou);
}

pub fn render_mmu_pane(ui: &mut egui::Ui, iou: &IouSnapshot) {
    egui::ScrollArea::vertical()
        .id_salt("cpu_mmu_pane")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            section_header(ui, "MMU");
            mmu_chips(ui, iou);
        });
}

fn input_summary(ui: &mut egui::Ui, iou: &IouSnapshot) {
    let dim = ui.visuals().weak_text_color();
    let on_col = egui::Color32::from_rgb(0xE5, 0xC0, 0x70);

    // Keyboard row.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("KBD").monospace().small().color(dim));

        let raw = iou.kbd_last_key;
        let ascii = raw & 0x7F;
        let glyph = if (0x20..=0x7E).contains(&ascii) {
            format!(" '{}'", ascii as char)
        } else {
            String::new()
        };
        ui.label(
            egui::RichText::new(format!("${:02X}{}", raw, glyph))
                .monospace().small()
                .color(if iou.kbd_strobe { on_col } else { dim }),
        );
        ui.label(
            egui::RichText::new(if iou.kbd_strobe { "STR" } else { "   " })
                .monospace().small().color(on_col),
        );
        ui.label(
            egui::RichText::new(format!("hld={} q={}", iou.kbd_held, iou.kbd_queued))
                .monospace().small().color(dim),
        );
    });

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("MSE").monospace().small().color(dim));
        ui.label(
            egui::RichText::new(format!(
                "X={:>4} Y={:>4}",
                iou.mouse_x, iou.mouse_y
            ))
            .monospace().small().color(dim),
        );
        ui.label(
            egui::RichText::new(if iou.mouse_button0 { "B0" } else { "  " })
                .monospace().small().color(on_col),
        );
        ui.label(
            egui::RichText::new(if iou.mouse_button1 { "B1" } else { "  " })
                .monospace().small().color(on_col),
        );
        for (label, on) in [
            ("Xi", iou.mouse_x_int),
            ("Yi", iou.mouse_y_int),
            ("Vi", iou.mouse_vbl_int),
        ] {
            ui.label(
                egui::RichText::new(if on { label } else { "  " })
                    .monospace().small().color(on_col),
            );
        }
    });
}

fn scc_summary(ui: &mut egui::Ui, iou: &IouSnapshot) {
    let dim = ui.visuals().weak_text_color();
    let on_col = egui::Color32::from_rgb(0xE5, 0xC0, 0x70);

    let render_row = |ui: &mut egui::Ui, label: &str,
                      ch: &crate::device::scc::SccChannelSnap| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).monospace().small().color(dim));

            let m = match ch.modem_state {
                0 => "CMD",
                1 => "ON ",
                _ => "ESC",
            };
            ui.label(
                egui::RichText::new(m)
                    .monospace().small()
                    .color(if ch.modem_state == 1 { on_col } else { dim }),
            );

            for (lbl, on) in [
                ("CONN", ch.connected),
                ("DCD",  ch.dcd),
                ("CTS",  ch.cts),
            ] {
                ui.label(
                    egui::RichText::new(lbl)
                        .monospace().small()
                        .color(if on { on_col } else { dim }),
                );
            }

            ui.label(
                egui::RichText::new(format!("rx={}", ch.rx_depth))
                    .monospace().small()
                    .color(if ch.rx_ready { on_col } else { dim }),
            );
            ui.label(
                egui::RichText::new(if ch.tx_empty { "TXE" } else { "txf" })
                    .monospace().small()
                    .color(if ch.tx_empty { on_col } else { dim }),
            );
            ui.label(
                egui::RichText::new(if ch.irq_pending { "IRQ" } else { "   " })
                    .monospace().small()
                    .color(egui::Color32::from_rgb(0xE5, 0x90, 0x60)),
            );
            if ch.loopback {
                ui.label(
                    egui::RichText::new("LB")
                        .monospace().small().color(on_col),
                );
            }
            if ch.baud > 0 {
                ui.label(
                    egui::RichText::new(format!("{}b", ch.baud))
                        .monospace().small().color(dim),
                );
            }
        });
    };

    render_row(ui, "ChA", &iou.scc_a);
    render_row(ui, "ChB", &iou.scc_b);
    if iou.scc_crossloop {
        ui.label(
            egui::RichText::new("crossloop A↔B")
                .monospace().small().color(on_col),
        );
    }
}

pub fn scan_position_bar(ui: &mut egui::Ui, iou: &IouSnapshot, show_cursor: bool) {
    let total = timing::CYCLES_PER_FRAME.max(1);
    let vbl_start = timing::VBL_START_CYCLE.min(total);
    let pos = iou.scan_cycle.min(total);
    let line = pos / timing::CYCLES_PER_SCANLINE;
    let cyc_in_line = pos % timing::CYCLES_PER_SCANLINE;
    let in_vbl = pos >= vbl_start;
    let dim = ui.visuals().weak_text_color();

    ui.horizontal(|ui| {
        let text_w = 132.0;
        let bar_w = (ui.available_width() - text_w).max(64.0);
        let height = 10.0;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(bar_w, height),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);

        let visible_col = egui::Color32::from_rgb(0x2A, 0x40, 0x2A);
        let vbl_col     = egui::Color32::from_rgb(0x40, 0x2A, 0x2A);
        let cursor_col  = egui::Color32::from_rgb(0xE5, 0xC0, 0x70);
        let border_col  = egui::Color32::from_rgb(0x33, 0x33, 0x40);

        let split_x = rect.left()
            + (rect.width() * vbl_start as f32 / total as f32);
        painter.rect_filled(
            egui::Rect::from_min_max(rect.left_top(), egui::pos2(split_x, rect.bottom())),
            1.0,
            visible_col,
        );
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(split_x, rect.top()), rect.right_bottom()),
            1.0,
            vbl_col,
        );
        painter.rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.0, border_col),
            egui::StrokeKind::Outside,
        );

        let x = rect.left() + rect.width() * pos as f32 / total as f32;
        if show_cursor {
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(2.0, cursor_col),
            );
        }

        if show_cursor {
            ui.label(
                egui::RichText::new(format!("L{:>3}.{:>2}", line, cyc_in_line))
                    .monospace().small().color(dim),
            );
        } else {
            ui.label(
                egui::RichText::new("—")
                    .monospace().small().color(dim),
            )
            .on_hover_text(
                "Scan position is only cycle-accurate after a Step or \
                 breakpoint halt. Free-running / plain Pause samples \
                 land on a frame boundary so the value would just drift.",
            );
        }
        if in_vbl && show_cursor {
            ui.label(
                egui::RichText::new("RETR")
                    .monospace().small().strong()
                    .color(egui::Color32::from_rgb(0xE5, 0x90, 0x60)),
            );
        }
    });
}

fn section_header(ui: &mut egui::Ui, label: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(label)
            .strong()
            .color(ui.visuals().strong_text_color()),
    );
}

pub fn video_chips(ui: &mut egui::Ui, iou: &IouSnapshot) {
    let v = iou.video_mode;
    ui.horizontal_wrapped(|ui| {
        flag_chip(ui, "TEXT",   v & VideoModeMask::TEXT != 0);
        flag_chip(ui, "LORES",  v & VideoModeMask::LORES != 0);
        flag_chip(ui, "HIRES",  v & VideoModeMask::HIRES != 0);
        flag_chip(ui, "DHIRES", v & VideoModeMask::DHIRES != 0);
        flag_chip(ui, "MIXED",  v & VideoModeMask::MIXED != 0);
        flag_chip(ui, "PAGE2",  v & VideoModeMask::PAGE2 != 0);
        flag_chip(ui, "COL80",  v & VideoModeMask::COL80 != 0);
        flag_chip(ui, "ALTCH",  v & VideoModeMask::ALTCHAR != 0);
        flag_chip(ui, "80STR",  iou.is_80store);
    });
}

pub fn mmu_chips(ui: &mut egui::Ui, iou: &IouSnapshot) {
    let s = iou.mem_state;
    ui.horizontal_wrapped(|ui| {
        flag_chip(ui, "ALTZP",  s & MemStateMask::ALTZP != 0);
        flag_chip(ui, "RAMRD",  s & MemStateMask::RAMRD != 0);
        flag_chip(ui, "RAMWRT", s & MemStateMask::RAMWRT != 0);
        flag_chip(ui, "LCRAM",  s & MemStateMask::LCRAM != 0);
        flag_chip(ui, "RDBNK2", s & MemStateMask::RDBNK != 0);
        flag_chip(ui, "WRITE",  s & MemStateMask::WRITE != 0);
        flag_chip(ui, "ALTROM", s & MemStateMask::ALTROM != 0);
        flag_chip(ui, "P2STR",  s & MemStateMask::P280STORE != 0);
    });
}
