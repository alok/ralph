import Std

namespace Ralph

structure Options where
  runner : String := "codex"
  model : String := "gpt-5.2-codex"
  reasoningEffort : String := "xhigh"
  iterations : Nat := 24
  sleepSec : Nat := 15
  promptTemplate : Option System.FilePath := none
  prd : Option System.FilePath := none
  progress : Option System.FilePath := none
  log : Option System.FilePath := none
  noLog : Bool := false
  stopToken : String := "__RALPH_DONE__"
  promptFlag : String := "-p"
  runnerArgs : List String := []
  resume : Bool := false
  resumeId : Option String := none
  fullAuto : Bool := false
  noYolo : Bool := false
  deriving Repr

/-- Fill in default paths relative to `cwd/ralph/` when none are provided. -/
def Options.resolvePaths (opts : Options) (cwd : System.FilePath) : Options :=
  let base := cwd / "ralph"
  let promptTemplate := opts.promptTemplate.getD (base / "prompt-template.md")
  let prd := opts.prd.getD (base / "PRD.md")
  let progress := opts.progress.getD (base / "progress.txt")
  let log := opts.log.getD (base / "overnight.log")
  { opts with
    promptTemplate := some promptTemplate
    prd := some prd
    progress := some progress
    log := some log }

end Ralph
