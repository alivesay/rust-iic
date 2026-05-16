# Builds 6502 modules under asm/ into build/asm/.
#
#   * Top-level `*.s` + `*.cfg`                          -> build/asm/<name>.bin
#   * `programs/<name>/<name>.s` + `<name>.cfg`          -> build/asm/<name>.bin   (same flat dir)
#                                                        -> build/asm/<name>.woz  (via mkdisk, see `make disks`)
#
# `make` (the default) only produces .bin files so build.rs can also drive it
# without recursive cargo invocation.  `make disks` additionally runs the
# already-built `target/debug/mkdisk` to wrap each program .bin in a bootable
# DOS 3.3 WOZ image.

ASM_DIR    := .
BUILD_DIR  := ../build/asm
CARGO_DIR  := ..

CA65 := ca65
LD65 := ld65

# Shared include path for program sources (lib/macros.inc, lib/hgr.inc, etc.)
LIB_DIR  := $(ASM_DIR)/lib
CA65_INC := -I $(LIB_DIR)

# --- Library code -----------------------------------------------------------
# Every program links lib/*.s + the program's own .o.
LIB_SRCS := $(wildcard $(LIB_DIR)/*.s)
LIB_OBJS := $(patsubst $(LIB_DIR)/%.s,$(BUILD_DIR)/lib_%.o,$(LIB_SRCS))

# --- Top-level modules ------------------------------------------------------
TOP_SRCS := $(wildcard $(ASM_DIR)/*.s)
TOP_BINS := $(patsubst $(ASM_DIR)/%.s,$(BUILD_DIR)/%.bin,$(TOP_SRCS))

# --- Programs (asm with optional code.bas) ----------------------------------
PROGRAM_DIRS  := $(wildcard $(ASM_DIR)/programs/*/.)
PROGRAM_NAMES := $(notdir $(patsubst %/.,%,$(PROGRAM_DIRS)))
PROGRAM_BINS  := $(foreach p,$(PROGRAM_NAMES),$(BUILD_DIR)/$(p).bin)
PROGRAM_WOZS  := $(foreach p,$(PROGRAM_NAMES),$(BUILD_DIR)/$(p).woz)

# --- BASIC-only games (just a code.bas; pack with the shared engine bin) ----
# Currently unused (rogue1 was the only consumer); structure left in place.
GAME_DIRS  :=
GAME_NAMES :=
GAME_WOZS  :=
ENGINE_BIN := $(BUILD_DIR)/engine.bin

MKDISK := $(CARGO_DIR)/target/debug/mkdisk
MKPODISK := $(CARGO_DIR)/target/debug/mkpodisk
POMVFILE := $(CARGO_DIR)/target/debug/pomvfile

.PHONY: all disks clean
all: $(TOP_BINS) $(PROGRAM_BINS)
disks: $(PROGRAM_WOZS)

# --- Top-level rules --------------------------------------------------------
$(BUILD_DIR)/%.o: $(ASM_DIR)/%.s | $(BUILD_DIR)
	@echo "🎯 Assembling $< -> $@"
	$(CA65) $< -o $@

$(BUILD_DIR)/%.bin: $(BUILD_DIR)/%.o $(ASM_DIR)/%.cfg | $(BUILD_DIR)
	@echo "🔗 Linking $< -> $@"
	$(LD65) -C $(ASM_DIR)/$*.cfg $< -o $@

# --- Library object rule (lib/foo.s -> build/asm/lib_foo.o) -----------------
$(BUILD_DIR)/lib_%.o: $(LIB_DIR)/%.s | $(BUILD_DIR)
	@echo "📚 Assembling lib $< -> $@"
	$(CA65) $(CA65_INC) $< -o $@

# --- Per-program rules ------------------------------------------------------
# A program is a dir under programs/ containing one .cfg and one or more .s
# files. Every .s in the dir is assembled (per-program prefix avoids name
# collisions in build/asm/) and linked together with all lib objs.
define PROGRAM_RULES
$(1)_SRCS := $$(wildcard $$(ASM_DIR)/programs/$(1)/*.s)
$(1)_OBJS := $$(patsubst $$(ASM_DIR)/programs/$(1)/%.s,$$(BUILD_DIR)/prog_$(1)_%.o,$$($(1)_SRCS))
$(1)_INCS := $$(wildcard $$(ASM_DIR)/programs/$(1)/*.inc)

$$(BUILD_DIR)/prog_$(1)_%.o: $$(ASM_DIR)/programs/$(1)/%.s $$($(1)_INCS) | $$(BUILD_DIR)
	@echo "🎯 Assembling $$< -> $$@"
	$$(CA65) $$(CA65_INC) -I $$(ASM_DIR)/programs/$(1) $$< -o $$@

$$(BUILD_DIR)/$(1).bin: $$($(1)_OBJS) $$(LIB_OBJS) $$(ASM_DIR)/programs/$(1)/$(1).cfg | $$(BUILD_DIR)
	@echo "🔗 Linking $(1) ($$(words $$($(1)_OBJS)) obj) + lib -> $$@"
	$$(LD65) -C $$(ASM_DIR)/programs/$(1)/$(1).cfg $$($(1)_OBJS) $$(LIB_OBJS) -o $$@

ifeq ($(filter $(1),cart_edit),)
$$(BUILD_DIR)/$(1).woz: $$(BUILD_DIR)/$(1).bin $$(MKDISK)
	@echo "💾 Packing $$< -> $$@"
	$$(MKDISK) --input $$< --output $$@ --load-addr 0x6000 \
		--name $$(shell echo $(1) | tr a-z A-Z | cut -c1-30)
endif
endef
$(foreach p,$(PROGRAM_NAMES),$(eval $(call PROGRAM_RULES,$(p))))

# --- cart_edit: standalone asm editor packaged with TILES / SFLAGS / MAP ----
# Each of those three is a separate ProDOS file BLOAD'd by STARTUP into RAM
# (TILES @ $4000, SFLAGS @ $6000, MAP01.MAP @ $6100). The save-loop BSAVEs
# all three back to disk whenever the editor sets $0BFF.

CARTEDIT_ASSETS := $(CARGO_DIR)/assets/cartedit
CARTEDIT_TILES  := $(CARTEDIT_ASSETS)/TILES.bin
CARTEDIT_SFLAGS := $(CARTEDIT_ASSETS)/SFLAGS.bin
CARTEDIT_MAP    := $(CARTEDIT_ASSETS)/MAP01.MAP

$(CARTEDIT_ASSETS):
	mkdir -p $@

$(CARTEDIT_TILES): | $(CARTEDIT_ASSETS)
	@if [ ! -f $@ ]; then \
		echo "✍  Seeding empty TILES (8 KB) -> $@"; \
		dd if=/dev/zero of=$@ bs=1024 count=8 status=none; \
	fi

$(CARTEDIT_SFLAGS): | $(CARTEDIT_ASSETS)
	@if [ ! -f $@ ]; then \
		echo "✍  Seeding empty SFLAGS (256 B) -> $@"; \
		dd if=/dev/zero of=$@ bs=256 count=1 status=none; \
	fi

$(CARTEDIT_MAP): | $(CARTEDIT_ASSETS)
	@if [ ! -f $@ ] || [ "$$(wc -c <$@)" -ne 5888 ]; then \
		echo "✍  Seeding empty MAP01.MAP (5888 B) -> $@"; \
		dd if=/dev/zero of=$@ bs=5888 count=1 status=none; \
	fi

.PHONY: cart_edit cart_edit_save_back
cart_edit: cart_edit_save_back $(BUILD_DIR)/cart_edit.po.woz

cart_edit_save_back: $(POMVFILE)
	@$(POMVFILE) --input $(BUILD_DIR)/cart_edit.po.woz \
		--out TILES:$(CARTEDIT_TILES) \
		--out SFLAGS:$(CARTEDIT_SFLAGS) \
		--out MAP01.MAP:$(CARTEDIT_MAP)
	@if [ -f $(CARTEDIT_MAP) ] && [ "$$(wc -c <$(CARTEDIT_MAP))" -ne 5888 ]; then \
		echo "✍  MAP01.MAP wrong size ($$(wc -c <$(CARTEDIT_MAP)) B); re-seeding to 5888 B (engine will init header on boot)"; \
		dd if=/dev/zero of=$(CARTEDIT_MAP) bs=5888 count=1 status=none; \
	fi

$(BUILD_DIR)/cart_edit.po.woz: $(BUILD_DIR)/cart_edit.bin $(CARTEDIT_TILES) $(CARTEDIT_SFLAGS) $(CARTEDIT_MAP) $(MKPODISK) $(CARGO_DIR)/assets/prodos/PRODOS.bin
	@echo "💿 Packing cart_edit ProDOS -> $@"
	$(MKPODISK) --input $(BUILD_DIR)/cart_edit.bin --output $@ --load-addr 0x7800 \
		--name CARTEDIT \
		--volume CARTEDIT \
		--prodos-bin $(CARGO_DIR)/assets/prodos/PRODOS.bin \
		--basic-system-bin $(CARGO_DIR)/assets/prodos/BASIC.SYSTEM.bin \
		--extra-file $(CARTEDIT_TILES):TILES:0x4000 \
		--extra-file $(CARTEDIT_SFLAGS):SFLAGS:0x6000 \
		--extra-file $(CARTEDIT_MAP):MAP01.MAP:0x6100 \
		--save-file TILES:0x4000:0x2000 \
		--save-file SFLAGS:0x6000:0x0100 \
		--save-file MAP01.MAP:0x6100:0x1700 \
		--save-loop

$(MKPODISK):
	@echo "🛠  Building mkpodisk (cargo build --bin mkpodisk)"
	cd $(CARGO_DIR) && cargo build --bin mkpodisk

$(POMVFILE):
	@echo "🛠  Building pomvfile (cargo build --bin pomvfile)"
	cd $(CARGO_DIR) && cargo build --bin pomvfile

$(MKDISK):
	@echo "🛠  Building mkdisk (cargo build --bin mkdisk)"
	cd $(CARGO_DIR) && cargo build --bin mkdisk

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

clean:
	rm -rf $(BUILD_DIR)
