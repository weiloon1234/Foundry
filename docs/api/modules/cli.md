# cli

CLI command registration (CommandRegistry)

[Back to index](../index.md)

## foundry::cli

```rust
pub type CommandRegistrar = Arc<dyn Fn(&mut CommandRegistry) -> Result<()> + Send + Sync>;
struct CommandInvocation
  fn app(&self) -> &AppContext
  fn matches(&self) -> &ArgMatches
struct CommandRegistry
  fn new() -> Self
  fn command<I, F, Fut>( &mut self, id: I, command: Command, handler: F, ) -> Result<&mut Self>
struct RegisteredCommand
```
