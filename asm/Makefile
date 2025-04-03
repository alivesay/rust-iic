# Directories
ASM_DIR := .
BUILD_DIR := ../build/asm

# Compiler and Linker
CA65 := ca65
LD65 := ld65

# Find all .s files
ASM_SRCS := $(wildcard $(ASM_DIR)/*.s)
OBJ_FILES := $(patsubst $(ASM_DIR)/%.s, $(BUILD_DIR)/%.o, $(ASM_SRCS))
BIN_FILES := $(patsubst $(ASM_DIR)/%.s, $(BUILD_DIR)/%.bin, $(ASM_SRCS))

# Default target: build everything
all: $(BIN_FILES)

# Rule to compile .s to .o
$(BUILD_DIR)/%.o: $(ASM_DIR)/%.s | $(BUILD_DIR)
	@echo "🎯 Assembling $< -> $@"
	$(CA65) $< -o $@

# Rule to link .o to .bin using corresponding .cfg
$(BUILD_DIR)/%.bin: $(BUILD_DIR)/%.o $(ASM_DIR)/%.cfg | $(BUILD_DIR)
	@echo "🔗 Linking $< -> $@"
	$(LD65) -C $(ASM_DIR)/$*.cfg $< -o $@

# Ensure build directory exists
$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

# Clean build artifacts
clean:
	rm -rf $(BUILD_DIR)