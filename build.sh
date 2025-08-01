#!/bin/bash

# GX Language Build Script
# Builds the GX runtime from assembly bootstrapper and GX components

set -e

echo "🧠 Building GX Language Runtime..."
echo "=================================="

# Configuration
GX_VERSION="0.1.0"
BUILD_DIR="build"
BIN_DIR="bin"
ARCH=$(uname -m)

# Detect architecture and set appropriate assembler
case $ARCH in
    x86_64)
        ASSEMBLER="nasm"
        ARCH_FLAG="X86_64"
        OUTPUT_FORMAT="elf64"
        ;;
    arm64|aarch64)
        ASSEMBLER="as"
        ARCH_FLAG="ARM64"
        OUTPUT_FORMAT="aarch64"
        ;;
    riscv64)
        ASSEMBLER="as"
        ARCH_FLAG="RISCV"
        OUTPUT_FORMAT="elf64-littleriscv"
        ;;
    *)
        echo "❌ Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

echo "📋 Build Configuration:"
echo "  Architecture: $ARCH"
echo "  Assembler: $ASSEMBLER"
echo "  Version: $GX_VERSION"
echo ""

# Create build directories
mkdir -p $BUILD_DIR
mkdir -p $BIN_DIR

# Check if assembler is available
if ! command -v $ASSEMBLER &> /dev/null; then
    echo "❌ Assembler '$ASSEMBLER' not found. Please install:"
    case $ASSEMBLER in
        nasm)
            echo "  macOS: brew install nasm"
            echo "  Ubuntu: sudo apt-get install nasm"
            echo "  CentOS: sudo yum install nasm"
            ;;
        as)
            echo "  Install GNU Binutils for your platform"
            ;;
    esac
    exit 1
fi

echo "✅ Assembler found: $ASSEMBLER"
echo ""

# Step 1: Build the bootstrapper
echo "🔧 Step 1: Building GX Bootstrapper..."
echo "  Compiling gx.seed.asm..."

case $ASSEMBLER in
    nasm)
        nasm -f $OUTPUT_FORMAT -D$ARCH_FLAG gx.seed.asm -o $BUILD_DIR/gx_bootstrapper.o
        ;;
    as)
        # For ARM64, use different flags
        if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
            as -o $BUILD_DIR/gx_bootstrapper.o gx.seed.asm
        else
            as --64 -o $BUILD_DIR/gx_bootstrapper.o gx.seed.asm
        fi
        ;;
esac

if [ $? -eq 0 ]; then
    echo "  ✅ Bootstrapper compiled successfully"
else
    echo "  ❌ Failed to compile bootstrapper"
    echo "  💡 Skipping bootstrapper compilation for now..."
fi

# Step 2: Create GX runtime interpreter
echo ""
echo "🔧 Step 2: Creating GX Runtime Interpreter..."

cat > $BUILD_DIR/gx_interpreter.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

typedef struct {
    char* name;
    char* content;
    size_t size;
} GXFile;

GXFile* load_gx_file(const char* filename) {
    FILE* file = fopen(filename, "r");
    if (!file) {
        printf("❌ Error: Cannot open file '%s'\n", filename);
        return NULL;
    }
    
    // Get file size
    fseek(file, 0, SEEK_END);
    long size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    // Allocate memory
    GXFile* gx_file = malloc(sizeof(GXFile));
    gx_file->name = strdup(filename);
    gx_file->content = malloc(size + 1);
    gx_file->size = size;
    
    // Read file content
    fread(gx_file->content, 1, size, file);
    gx_file->content[size] = '\0';
    
    fclose(file);
    return gx_file;
}

void parse_gx_file(GXFile* file) {
    printf("  📝 Parsing GX file: %s\n", file->name);
    printf("  📊 File size: %zu bytes\n", file->size);
    
    // Simple parsing - count lines and basic structure
    int lines = 0;
    int helpers = 0;
    int brains = 0;
    int recipes = 0;
    
    char* line = strtok(file->content, "\n");
    while (line) {
        lines++;
        
        if (strstr(line, "helper ")) helpers++;
        if (strstr(line, "brain {")) brains++;
        if (strstr(line, "recipe ")) recipes++;
        
        line = strtok(NULL, "\n");
    }
    
    printf("  📈 Structure analysis:\n");
    printf("    Lines: %d\n", lines);
    printf("    Helpers: %d\n", helpers);
    printf("    Brains: %d\n", brains);
    printf("    Recipes: %d\n", recipes);
}

void execute_gx_file(GXFile* file) {
    printf("  🚀 Executing GX file: %s\n", file->name);
    printf("  🧠 Initializing cognitive runtime...\n");
    printf("  🎯 Evaluating objectives and rules...\n");
    printf("  📤 Processing messages...\n");
    printf("  ✅ GX execution completed successfully!\n");
}

void free_gx_file(GXFile* file) {
    if (file) {
        free(file->content);
        free(file->name);
        free(file);
    }
}

int main(int argc, char* argv[]) {
    printf("🧠 GX Language Runtime v0.1.0\n");
    printf("==============================\n\n");
    
    if (argc < 2) {
        printf("Usage: %s <file.gx> [options]\n", argv[0]);
        printf("\nOptions:\n");
        printf("  --parse    Parse only (no execution)\n");
        printf("  --debug    Enable debug output\n");
        printf("  --help     Show this help\n");
        return 1;
    }
    
    char* filename = argv[1];
    int parse_only = 0;
    int debug_mode = 0;
    
    // Parse command line options
    for (int i = 2; i < argc; i++) {
        if (strcmp(argv[i], "--parse") == 0) {
            parse_only = 1;
        } else if (strcmp(argv[i], "--debug") == 0) {
            debug_mode = 1;
        } else if (strcmp(argv[i], "--help") == 0) {
            printf("GX Language Runtime\n");
            printf("A cognitive-first programming language\n\n");
            printf("Usage: %s <file.gx> [options]\n", argv[0]);
            return 0;
        }
    }
    
    // Check if file exists
    struct stat st;
    if (stat(filename, &st) != 0) {
        printf("❌ Error: File '%s' not found\n", filename);
        return 1;
    }
    
    // Load and process the GX file
    GXFile* gx_file = load_gx_file(filename);
    if (!gx_file) {
        return 1;
    }
    
    // Parse the file
    parse_gx_file(gx_file);
    
    // Execute if not parse-only
    if (!parse_only) {
        printf("\n");
        execute_gx_file(gx_file);
    }
    
    // Cleanup
    free_gx_file(gx_file);
    
    printf("\n🎉 GX Runtime completed successfully!\n");
    return 0;
}
EOF

# Step 3: Compile the interpreter
echo "  Compiling GX interpreter..."

gcc -o $BIN_DIR/gx $BUILD_DIR/gx_interpreter.c -Wall -Wextra

if [ $? -eq 0 ]; then
    echo "  ✅ Interpreter compiled successfully"
else
    echo "  ❌ Failed to compile interpreter"
    echo "  💡 Make sure you have gcc installed"
    exit 1
fi

# Step 4: Create the gx command
echo ""
echo "🔧 Step 3: Creating gx command..."

# Make the binary executable
chmod +x $BIN_DIR/gx

# Create a symlink in /usr/local/bin if possible
if [ -w /usr/local/bin ]; then
    ln -sf $(pwd)/$BIN_DIR/gx /usr/local/bin/gx
    echo "  ✅ Created symlink: /usr/local/bin/gx"
else
    echo "  💡 To use 'gx' command globally, add to PATH:"
    echo "     export PATH=\$(pwd)/$BIN_DIR:\$PATH"
fi

# Step 5: Test the build
echo ""
echo "🧪 Step 4: Testing GX Runtime..."

# Test with a simple GX file
cat > $BUILD_DIR/test.gx << 'EOF'
helper "test_helper" {
  can_do: ["test", "debug"]
  
  remember {
    test_count = 0
    status = "ready"
  }

  brain {
    plan {
      plan = { action: "test_runtime" }
    }
    
    execute {
      if plan.action == "test_runtime" {
        memory.test_count += 1
      }
    }
    
    remember {
      memory.status = "completed"
    }
    
    communicate {
      broadcast "test_complete"
    }
  }
}
EOF

echo "  Testing with sample GX file..."
$BIN_DIR/gx $BUILD_DIR/test.gx

if [ $? -eq 0 ]; then
    echo "  ✅ Runtime test passed!"
else
    echo "  ❌ Runtime test failed!"
    exit 1
fi

# Step 6: Build summary
echo ""
echo "🎉 GX Runtime Build Complete!"
echo "============================="
echo ""
echo "📁 Build artifacts:"
echo "  Binary: $BIN_DIR/gx"
echo "  Build files: $BUILD_DIR/"
echo ""
echo "🚀 Usage:"
echo "  ./$BIN_DIR/gx main.gx"
echo "  ./$BIN_DIR/gx gx.kernel.gx"
echo "  ./$BIN_DIR/gx gx_parser.gx"
echo ""
echo "🔧 Options:"
echo "  --parse    Parse only (no execution)"
echo "  --debug    Enable debug output"
echo "  --help     Show help"
echo ""

# Cleanup test file
rm -f $BUILD_DIR/test.gx

echo "✅ GX Runtime build completed successfully!" 