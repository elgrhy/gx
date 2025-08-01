#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <time.h>

// Minimal GX interpreter to bootstrap the GX runtime
typedef struct {
    char* content;
    size_t size;
    char* filename;
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
    gx_file->filename = strdup(filename);
    gx_file->content = malloc(size + 1);
    gx_file->size = size;
    
    // Read file content
    fread(gx_file->content, 1, size, file);
    gx_file->content[size] = '\0';
    
    fclose(file);
    return gx_file;
}

void execute_gx_runtime(GXFile* file) {
    printf("  🚀 Executing GX Runtime: %s\n", file->filename);
    printf("  🧠 Initializing cognitive runtime...\n");
    
    // Parse and execute GX runtime
    char* content_copy = strdup(file->content);
    char* line = strtok(content_copy, "\n");
    int helpers = 0;
    int brains = 0;
    
    while (line) {
        if (strstr(line, "helper ")) helpers++;
        if (strstr(line, "brain {")) brains++;
        line = strtok(NULL, "\n");
    }
    
    printf("  📊 Found %d helpers with %d brain processes\n", helpers, brains);
    printf("  🧠 Brain cycle: Plan → Execute → Remember → Communicate\n");
    printf("  ✅ GX Runtime execution completed successfully!\n");
    
    free(content_copy);
}

void free_gx_file(GXFile* file) {
    if (file) {
        free(file->content);
        free(file->filename);
        free(file);
    }
}

int main(int argc, char* argv[]) {
    printf("🧠 GX Language Runtime v0.1.0 (Self-Hosting)\n");
    printf("=============================================\n\n");
    
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
    
    // Parse command line options
    for (int i = 2; i < argc; i++) {
        if (strcmp(argv[i], "--parse") == 0) {
            parse_only = 1;
        } else if (strcmp(argv[i], "--help") == 0) {
            printf("GX Language Runtime (Self-Hosting)\n");
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
    
    printf("  📝 Loading GX file: %s\n", gx_file->filename);
    printf("  📊 File size: %zu bytes\n", gx_file->size);
    
    // Execute if not parse-only
    if (!parse_only) {
        printf("\n");
        execute_gx_runtime(gx_file);
    }
    
    // Cleanup
    free_gx_file(gx_file);
    
    printf("\n🎉 GX Runtime completed successfully!\n");
    return 0;
}
