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

# --- Top-level modules ------------------------------------------------------
TOP_SRCS := $(wildcard $(ASM_DIR)/*.s)
TOP_BINS := $(patsubst $(ASM_DIR)/%.s,$(BUILD_DIR)/%.bin,$(TOP_SRCS))

# --- Programs ---------------------------------------------------------------
PROGRAM_DIRS  := $(wildcard $(ASM_DIR)/programs/*/.)
PROGRAM_NAMES := $(notdir $(patsubst %/.,%,$(PROGRAM_DIRS)))
PROGRAM_BINS  := $(foreach p,$(PROGRAM_NAMES),$(BUILD_DIR)/$(p).bin)
PROGRAM_WOZS  := $(foreach p,$(PROGRAM_NAMES),$(BUILD_DIR)/$(p).woz)

MKDISK := $(CARGO_DIR)/target/debug/mkdisk

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

# --- Per-program rules (one .s + .cfg per dir) ------------------------------
define PROGRAM_RULES
$$(BUILD_DIR)/$(1).o: $$(ASM_DIR)/programs/$(1)/$(1).s | $$(BUILD_DIR)
	@echo "🎯 Assembling $$< -> $$@"
	$$(CA65) $$(CA65_INC) $$< -o $$@

$$(BUILD_DIR)/$(1).bin: $$(BUILD_DIR)/$(1).o $$(ASM_DIR)/programs/$(1)/$(1).cfg | $$(BUILD_DIR)
	@echo "🔗 Linking $$< -> $$@"
	$$(LD65) -C $$(ASM_DIR)/programs/$(1)/$(1).cfg $$< -o $$@

$$(BUILD_DIR)/$(1).woz: $$(BUILD_DIR)/$(1).bin $$(MKDISK)
	@echo "💾 Packing $$< -> $$@"
	$$(MKDISK) --input $$< --output $$@ --load-addr 0x6000 --name $$(shell echo $(1) | tr a-z A-Z | cut -c1-30)
endef
$(foreach p,$(PROGRAM_NAMES),$(eval $(call PROGRAM_RULES,$(p))))

$(MKDISK):
	@echo "🛠  Building mkdisk (cargo build --bin mkdisk)"
	cd $(CARGO_DIR) && cargo build --bin mkdisk

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

clean:
	rm -rf $(BUILD_DIR)
