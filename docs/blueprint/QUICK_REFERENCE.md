# GX Quick Reference

> Full reference: [../API_REFERENCE.md](../API_REFERENCE.md)

```bash
gx run file.gx           # Run
gx check file.gx         # Syntax check
gx init my-project       # New project
gx build file.gx         # Build launcher
gx install js.axios      # Install npm package
gx install py.requests   # Install Python package
gx fmt file.gx           # Format
gx test                  # Run tests
gx make "description"    # AI-generate code
gx version               # Version
gx help                  # Help
```

```gx
// Minimal working agent
agent "hello" {
  when started { say "Hello, World!" }
  brain { plan {} execute {} remember {} communicate {} }
}
```
