# Builds 6502 modules under asm/ into build/asm/.
#
#   * Top-level `*.s` + `*.cfg`               -> build/asm/<name>.bin
#   * `games/<name>/<name>.s` + `<name>.cfg`  -> build/asm/<name>.bin   (same flat dir)
#                                              -> build/asm/<name>.woz  (via mkdisk, see `make disks`)
#
# `make` (the default) only produces .bin files so build.rs can also drive it
# without recursive cargo invocation.  `make disks` additionally runs the
# already-built `target/debug/mkdisk` to wrap each game .bin in a bootable
# DOS 3.3 WOZ image.

ASM_DIR    := .
BUILD_DIR  := ../build/asm
CARGO_DIR  := ..

CA65 := ca65
LD65 := ld65

# --- Top-level modules ------------------------------------------------------
TOP_SRCS := $(wildcard $(ASM_DIR)/*.s)
TOP_BINS := $(patsubst $(ASM_DIR)/%.s,$(BUILD_DIR)/%.bin,$(TOP_SRCS))

# --- Games ------------------------------------------------------------------
GAME_DIRS  := $(wildcard $(ASM_DIR)/games/*/.)
GAME_NAMES := $(notdir $(patsubst %/.,%,$(GAME_DIRS)))
GAME_BINS  := $(foreach g,$(GAME_NAMES),$(BUILD_DIR)/$(g).bin)
GAME_WOZS  := $(foreach g,$(GAME_NAMES),$(BUILD_DIR)/$(g).woz)

MKDISK := $(CARGO_DIR)/target/debug/mkdisk

.PHONY: all disks clean
all: $(TOP_BINS) $(GAME_BINS)
disks: $(GAME_WOZS)

# --- Top-level rules --------------------------------------------------------
$(BUILD_DIR)/%.o: $(ASM_DIR)/%.s | $(BUILD_DIR)
	@echo "🎯 Assembling $< -> $@"
	$(CA65) $< -o $@

$(BUILD_DIR)/%.bin: $(BUILD_DIR)/%.o $(ASM_DIR)/%.cfg | $(BUILD_DIR)
	@echo "🔗 Linking $< -> $@"
	$(LD65) -C $(ASM_DIR)/$*.cfg $< -o $@

# --- Per-game rules (one .s + .cfg per dir) ---------------------------------
define GAME_RULES
$$(BUILD_DIR)/$(1).o: $$(ASM_DIR)/games/$(1)/$(1).s | $$(BUILD_DIR)
	@echo "🎯 Assembling $$< -> $$@"
	$$(CA65) $$< -o $$@

$$(BUILD_DIR)/$(1).bin: $$(BUILD_DIR)/$(1).o $$(ASM_DIR)/games/$(1)/$(1).cfg | $$(BUILD_DIR)
	@echo "🔗 Linking $$< -> $$@"
	$$(LD65) -C $$(ASM_DIR)/games/$(1)/$(1).cfg $$< -o $$@

$$(BUILD_DIR)/$(1).woz: $$(BUILD_DIR)/$(1).bin $$(MKDISK)
	@echo "💾 Packing $$< -> $$@"
	$$(MKDISK) --input $$< --output $$@ --load-addr 0x6000 --name $$(shell echo $(1) | tr a-z A-Z | cut -c1-30)
endef
$(foreach g,$(GAME_NAMES),$(eval $(call GAME_RULES,$(g))))

$(MKDISK):
	@echo "🛠  Building mkdisk (cargo build --bin mkdisk)"
	cd $(CARGO_DIR) && cargo build --bin mkdisk

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

clean:
	rm -rf $(BUILD_DIR)
