import Std
import Ralph.Options
import Ralph.Parse
import Ralph.Prompt
import Ralph.Runner
import Ralph.State
import Ralph.Log

open Ralph

private def usage : String :=
  String.intercalate "\n"
    [ "Permissive Ralph loop runner (Lean port, WIP)"
    , ""
    , "Usage: ralph [OPTIONS]"
    , ""
    , "Options:"
    , "  --runner <RUNNER>             [default: codex]"
    , "  --model <MODEL>               [default: gpt-5.2-codex]"
    , "  --reasoning-effort <EFFORT>   [default: xhigh]"
    , "  --iterations <N>              [default: 24]"
    , "  --sleep <SECONDS>             [default: 15]"
    , "  --prompt-template <PATH>"
    , "  --prd <PATH>"
    , "  --progress <PATH>"
    , "  --log <PATH>"
    , "  --no-log"
    , "  --stop-token <TOKEN>          [default: __RALPH_DONE__]"
    , "  --prompt-flag <FLAG>          [default: -p]"
    , "  --runner-arg <ARG>            (repeatable)"
    , "  --resume | --resume-id <ID>"
    , "  --full-auto"
    , "  --no-yolo"
    ]

private def ensureFile (path : System.FilePath) (label : String) : IO (Option UInt32) := do
  if !(← path.pathExists) then
    IO.eprintln s!"Missing {label}: {path}"
    return some 1
  return none

partial def runLoop (opts : Options) (prompt : String) : IO UInt32 := do
  let init := LoopState.start opts.iterations
  let st := LoopState.begin init
  go st
where
  go (st : LoopState .running) : IO UInt32 := do
    let iter := st.iter + 1
    IO.println s!"[ralph-lean] iteration {iter}/{st.max}"
    let output ← runCodex opts prompt
    if output.stdout != "" then
      IO.print output.stdout
    if output.stderr != "" then
      IO.eprint output.stderr
    if !opts.noLog then
      if let some logPath := opts.log then
        appendLog logPath iter output.stdout output.stderr output.exitCode
    if output.exitCode != 0 then
      return output.exitCode
    if output.stdout.toSlice.contains opts.stopToken then
      IO.println "[ralph-lean] completion detected, stopping."
      return 0
    let st' := LoopState.step st
    if st'.shouldContinue then
      IO.sleep (UInt32.ofNat opts.sleepSec)
      go st'
    else
      let _done := LoopState.finish st'
      return 0

/-- Lean port entrypoint. -/
def main (args : List String) : IO UInt32 := do
  let opts ←
    match parseArgs args with
    | .ok opts => pure opts
    | .error "help" =>
        IO.println usage
        return 0
    | .error msg =>
        IO.eprintln msg
        IO.eprintln usage
        return 1
  let cwd ← IO.currentDir
  let opts := opts.resolvePaths cwd
  if opts.runner != "codex" then
    IO.eprintln "Lean port currently supports only the codex runner."
    return 2
  let some templatePath := opts.promptTemplate
    | return 1
  let some prdPath := opts.prd
    | return 1
  let some progressPath := opts.progress
    | return 1
  if let some code := (← ensureFile templatePath "prompt template") then
    return code
  if let some code := (← ensureFile prdPath "PRD") then
    return code
  if let some code := (← ensureFile progressPath "progress log") then
    return code
  let prompt ← loadPrompt templatePath prdPath progressPath
  runLoop opts prompt
