#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum InterruptType {
    NMI,
    IRQ,
    BRK,
    RST,
}

#[derive(Default)]
pub struct InterruptController {
    pub nmi: bool,     // Non-Maskable Interrupt
    pub irq: bool,     // Maskable Interrupt
    pub brk: bool,     // Software Interrupt (BRK)
    pub reset: bool,   // Reset Interrupt
    pub waiting: bool, // WAI: CPU waiting for interrupt
    pub halted: bool,  // STP: CPU halted indefinitely
}

impl InterruptController {
    // pub fn request_reset(&mut self) {
    //     self.reset = true;
    //     self.waiting = false;
    //     self.halted = false;
    // }

    pub fn request_nmi(&mut self) {
        println!("NMI Requested");
        self.nmi = true;
        self.waiting = false;
    }

    pub fn request_irq(&mut self) {
        println!("IRQ Requested");
        self.irq = true;
        self.waiting = false;
    }

    pub fn request_brk(&mut self) {
        self.brk = true;
    }

    pub fn enter_wait(&mut self) {
        self.waiting = true;
    }

    pub fn leave_wait(&mut self) {
        self.waiting = false;
    }

    pub fn enter_halt(&mut self) {
        self.halted = true;
    }

    pub fn leave_halt(&mut self) {
        self.halted = false;
    }

    pub fn clear_all(&mut self) {
        self.nmi = false;
        self.irq = false;
        self.brk = false;
        self.reset = false;
    }

    pub fn handle_interrupt_with_vectors(
        &mut self,
        nmi_vector: u16,
        reset_vector: u16,
        irq_vector: u16,
    ) -> Option<(InterruptType, u16)> {
        if self.halted {
            return None;
        }

        let (interrupt_type, resolved_vector) = if self.nmi {
            (InterruptType::NMI, nmi_vector)
        } else if self.reset {
            self.reset = false;
            (InterruptType::RST, reset_vector)
        } else if self.brk {
            self.brk = false;
            (InterruptType::BRK, irq_vector)
        } else if self.irq {
            (InterruptType::IRQ, irq_vector)
        } else {
            return None;
        };

        println!(
            "Handling {:?} Interrupt: Jumping to {:#06X}",
            interrupt_type, resolved_vector
        );

        Some((interrupt_type, resolved_vector))
    }

    pub fn status_string(&self) -> String {
        format!(
            "I:{}{}{}{}{}{}",
            if self.nmi { "N" } else { "." },
            if self.irq { "I" } else { "." },
            if self.brk { "B" } else { "." },
            if self.reset { "R" } else { "." },
            if self.waiting { "W" } else { "." },
            if self.halted { "H" } else { "." },
        )
    }
}
