import Std
import Ralph.Options

namespace Ralph

/-- Run Codex in non-interactive mode with the provided prompt. -/
def runCodex (opts : Options) (prompt : String) : IO IO.Process.Output := do
  let mut args : Array String := #[]
  if opts.model != "" then
    args := args.push "--model" |>.push opts.model
  if opts.reasoningEffort != "" then
    args := args.push "-c" |>.push s!"model_reasoning_effort={opts.reasoningEffort}"
  if !opts.noYolo then
    args := args.push "--dangerously-bypass-approvals-and-sandbox"
  else if opts.fullAuto then
    args := args.push "--full-auto"
  args := args.push "exec"
  if opts.resume || opts.resumeId.isSome then
    args := args.push "resume"
    if let some id := opts.resumeId then
      args := args.push id
    else
      args := args.push "--last"
  if !opts.runnerArgs.isEmpty then
    args := args ++ opts.runnerArgs.toArray
  args := args.push "-"
  IO.Process.output { cmd := "codex", args } (some prompt)

end Ralph
