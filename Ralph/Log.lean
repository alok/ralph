import Std

namespace Ralph

/-- Append a simple iteration log. -/
def appendLog (logPath : System.FilePath) (iteration : Nat) (out err : String) (exitCode : UInt32) : IO Unit := do
  let parent? := logPath.parent
  match parent? with
  | some parent =>
      if !(← parent.pathExists) then
        IO.FS.createDirAll parent
  | none => pure ()
  let handle ← IO.FS.Handle.mk logPath .append
  handle.putStrLn s!"[iteration {iteration}]"
  if out.trimAscii.toString != "" then
    handle.putStrLn "\n[stdout]"
    handle.putStr out
  if err.trimAscii.toString != "" then
    handle.putStrLn "\n[stderr]"
    handle.putStr err
  handle.putStrLn s!"\n[exit-code] {exitCode}"
  let rule := String.ofList (List.replicate 80 '-')
  handle.putStrLn s!"\n{rule}"
  handle.flush
  pure ()

end Ralph
