import Std
import Cli
import Ralph.Options
import Ralph.Prompt
import Ralph.Runner
import Ralph.State
import Ralph.Log
import Ralph.Linear

open Ralph
open Cli

partial def runLoop (opts : Options) (prompt : String) : IO UInt32 := do
  let startMs ← IO.monoMsNow
  let init := LoopState.start opts.iterations
  let st := LoopState.begin init
  go startMs st
where
  go (startMs : Nat) (st : LoopState .running) : IO UInt32 := do
    if opts.maxSeconds > 0 then
      let nowMs ← IO.monoMsNow
      let elapsedMs := nowMs - startMs
      if elapsedMs >= opts.maxSeconds * 1000 then
        IO.println s!"[ralph-lean] stop: reached max runtime ({opts.maxSeconds}s)."
        return 0
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
      IO.println s!"[ralph-lean] stop: runner exited with {output.exitCode}."
      return output.exitCode
    if output.stdout.toSlice.contains opts.stopToken then
      IO.println "[ralph-lean] stop: completion token detected."
      return 0
    let st' := LoopState.step st
    if st'.shouldContinue then
      IO.sleep (UInt32.ofNat opts.sleepSec)
      go startMs st'
    else
      let _done := LoopState.finish st'
      IO.println "[ralph-lean] stop: reached max iterations."
      return 0

private def ensureFile (path : System.FilePath) (label : String) : IO (Option UInt32) := do
  if !(← path.pathExists) then
    IO.eprintln s!"Missing {label}: {path}"
    return some 1
  return none

private def optsFromParsed (p : Parsed) : Options :=
  let runner : String := p.flag! "runner" |>.as! String
  let model : String := p.flag! "model" |>.as! String
  let reasoningEffort : String := p.flag! "reasoning-effort" |>.as! String
  let iterations : Nat := p.flag! "iterations" |>.as! Nat
  let sleepSec : Nat := p.flag! "sleep" |>.as! Nat
  let maxSeconds : Nat := p.flag! "max-seconds" |>.as! Nat
  let stopToken : String := p.flag! "stop-token" |>.as! String
  let promptFlag : String := p.flag! "prompt-flag" |>.as! String
  let runnerArgs :=
    p.flags.toList.foldl
      (fun acc flag =>
        if flag.flag.longName == "runner-arg" then
          acc ++ (flag.as! (Array String)).toList
        else
          acc)
      []
  let resume := p.hasFlag "resume" || p.hasFlag "resume-id"
  let resumeId :=
    match p.flag? "resume-id" with
    | some flag => some (flag.as! String)
    | none => none
  let promptTemplate :=
    match p.flag? "prompt-template" with
    | some flag => some (System.FilePath.mk (flag.as! String))
    | none => none
  let prd :=
    match p.flag? "prd" with
    | some flag => some (System.FilePath.mk (flag.as! String))
    | none => none
  let progress :=
    match p.flag? "progress" with
    | some flag => some (System.FilePath.mk (flag.as! String))
    | none => none
  let log :=
    match p.flag? "log" with
    | some flag => some (System.FilePath.mk (flag.as! String))
    | none => none
  let extra :=
    match p.flag? "extra" with
    | some flag => some (flag.as! String)
    | none => none
  {
    runner,
    model,
    reasoningEffort,
    iterations,
    sleepSec,
    maxSeconds,
    promptTemplate,
    prd,
    progress,
    log,
    noLog := p.hasFlag "no-log",
    stopToken,
    promptFlag,
    runnerArgs,
    resume,
    resumeId,
    fullAuto := p.hasFlag "full-auto",
    noYolo := p.hasFlag "no-yolo",
    noLinear := p.hasFlag "no-linear",
    extra,
  }

def runRalph (p : Parsed) : IO UInt32 := do
  let mut opts := optsFromParsed p
  let cwd ← IO.currentDir
  opts := opts.resolvePaths cwd
  if opts.runner != "codex" then
    IO.eprintln "Lean port currently supports only the codex runner."
    return 2
  let syncInfo? ← syncLinearPRD opts
  if !opts.noLinear && syncInfo?.isNone then
    IO.println "[ralph-lean] Linear sync not available; continuing without it."
  let some templatePath := opts.promptTemplate | return 1
  let some prdPath := opts.prd | return 1
  let some progressPath := opts.progress | return 1
  if let some code := (← ensureFile templatePath "prompt template") then
    return code
  if let some code := (← ensureFile prdPath "PRD") then
    return code
  if let some code := (← ensureFile progressPath "progress log") then
    return code
  let linearExtra :=
    match syncInfo? with
    | some info =>
        s!"[linear-auto] Project: {info.project.name} ({info.project.url})\n" ++
        s!"[linear-auto] PRD doc: {info.doc.title} ({info.doc.url})"
    | none => ""
  let extra :=
    match opts.extra with
    | some text =>
        if linearExtra == "" then text else s!"{linearExtra}\n\n{text}"
    | none => linearExtra
  let prompt ← loadPrompt templatePath prdPath progressPath (some extra)
  runLoop opts prompt

def ralphCmd : Cmd := `[Cli|
  ralph VIA runRalph; ["0.1.0"]
  "Permissive Ralph loop runner (Lean port, WIP)."

  FLAGS:
    runner : String;           "Runner command (default: codex)."
    model : String;            "Model name (default: gpt-5.2-codex)."
    "reasoning-effort" : String; "Reasoning effort (default: xhigh)."
    iterations : Nat;          "Iterations to run (default: 24)."
    sleep : Nat;               "Sleep seconds between iterations (default: 15)."
    "max-seconds" : Nat;       "Stop after N seconds (0 = no limit)."
    "prompt-template" : String; "Prompt template path."
    prd : String;              "PRD path."
    progress : String;         "Progress log path."
    log : String;              "Run log path."
    "no-log";                  "Disable log append."
    "stop-token" : String;     "Stop token string."
    "prompt-flag" : String;    "Runner prompt flag."
    "runner-arg" : Array String; "Extra runner args (repeatable or comma-separated)."
    extra : String;            "Extra instructions to prepend to the prompt."
    resume;                    "Resume most recent codex session."
    "resume-id" : String;      "Resume specific codex session id."
    "full-auto";               "Use codex --full-auto when yolo disabled."
    "no-yolo";                 "Disable codex --yolo flag."
    "no-linear";               "Disable Linear sync."

  EXTENSIONS:
    defaultValues! #[
      ("runner", "codex"),
      ("model", "gpt-5.2-codex"),
      ("reasoning-effort", "xhigh"),
      ("iterations", "24"),
      ("sleep", "15"),
      ("max-seconds", "0"),
      ("stop-token", "__RALPH_DONE__"),
      ("prompt-flag", "-p")
    ]
]

/-- Lean port entrypoint. -/
def main (args : List String) : IO UInt32 :=
  ralphCmd.validate args
