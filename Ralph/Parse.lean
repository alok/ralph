import Std
import Ralph.Options

namespace Ralph

private def parseNat (flag value : String) : Except String Nat :=
  match value.toNat? with
  | some n => .ok n
  | none => .error s!"Invalid {flag}: {value}"

partial def parseArgs (args : List String) (opts : Options := {}) : Except String Options :=
  match args with
  | [] => .ok opts
  | "--runner" :: v :: rest =>
      parseArgs rest { opts with runner := v }
  | "--model" :: v :: rest =>
      parseArgs rest { opts with model := v }
  | "--reasoning-effort" :: v :: rest =>
      parseArgs rest { opts with reasoningEffort := v }
  | "--iterations" :: v :: rest =>
      match parseNat "--iterations" v with
      | .ok n => parseArgs rest { opts with iterations := n }
      | .error e => .error e
  | "--sleep" :: v :: rest =>
      match parseNat "--sleep" v with
      | .ok n => parseArgs rest { opts with sleepSec := n }
      | .error e => .error e
  | "--prompt-template" :: v :: rest =>
      parseArgs rest { opts with promptTemplate := some (System.FilePath.mk v) }
  | "--prd" :: v :: rest =>
      parseArgs rest { opts with prd := some (System.FilePath.mk v) }
  | "--progress" :: v :: rest =>
      parseArgs rest { opts with progress := some (System.FilePath.mk v) }
  | "--log" :: v :: rest =>
      parseArgs rest { opts with log := some (System.FilePath.mk v) }
  | "--no-log" :: rest =>
      parseArgs rest { opts with noLog := true }
  | "--stop-token" :: v :: rest =>
      parseArgs rest { opts with stopToken := v }
  | "--prompt-flag" :: v :: rest =>
      parseArgs rest { opts with promptFlag := v }
  | "--runner-arg" :: v :: rest =>
      parseArgs rest { opts with runnerArgs := opts.runnerArgs ++ [v] }
  | "--resume" :: rest =>
      parseArgs rest { opts with resume := true }
  | "--resume-id" :: v :: rest =>
      parseArgs rest { opts with resume := true, resumeId := some v }
  | "--full-auto" :: rest =>
      parseArgs rest { opts with fullAuto := true }
  | "--no-yolo" :: rest =>
      parseArgs rest { opts with noYolo := true }
  | "-h" :: _ =>
      .error "help"
  | "--help" :: _ =>
      .error "help"
  | flag :: _ =>
      .error s!"Unknown arg: {flag}"

end Ralph
