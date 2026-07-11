# cli

CLI command registration (CommandRegistry)

[Back to index](../index.md)

## foundry::cli

```rust
pub type CommandRegistrar = Arc<dyn Fn(&mut CommandRegistry) -> Result<()> + Send + Sync>;
struct CommandExit
  const SUCCESS: Self
  const FAILURE: Self
  const fn new(code: u8) -> Self
  const fn code(self) -> u8
  const fn is_success(self) -> bool
struct CommandInvocation
  fn app(&self) -> &AppContext
  fn matches(&self) -> &ArgMatches
  fn io(&self) -> &dyn CommandIo
  fn write(&self, message: impl AsRef<str>) -> Result<()>
  fn line(&self, message: impl AsRef<str>) -> Result<()>
  fn error(&self, message: impl AsRef<str>) -> Result<()>
  fn prompt(&self, question: impl AsRef<str>) -> Result<String>
  fn confirm(&self, question: impl AsRef<str>, default: bool) -> Result<bool>
  fn progress( &self, label: impl Into<String>, total: u64, ) -> Result<CommandProgress>
struct CommandProgress
  fn current(&self) -> u64
  fn total(&self) -> u64
  fn advance(&mut self, amount: u64) -> Result<()>
  fn finish(&mut self) -> Result<()>
  fn is_finished(&self) -> bool
struct CommandRegistry
  fn new() -> Self
  fn command<I, F, Fut>( &mut self, id: I, command: Command, handler: F, ) -> Result<&mut Self>
  fn command_with_exit<I, F, Fut>( &mut self, id: I, command: Command, handler: F, ) -> Result<&mut Self>
struct RegisteredCommand
struct TerminalCommandIo
trait CommandIo
  fn write_stdout(&self, message: &str) -> Result<()>
  fn write_stderr(&self, message: &str) -> Result<()>
  fn read_stdin_line(&self) -> Result<String>
```
