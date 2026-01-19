import Std

namespace Ralph

/-- Load a prompt template and substitute PRD/progress references. -/
def loadPrompt (templatePath prdPath progressPath : System.FilePath) (extra? : Option String := none) : IO String := do
  let template ← IO.FS.readFile templatePath
  let prdRef := s!"@{prdPath.toString}"
  let progressRef := s!"@{progressPath.toString}"
  let prompt := template.replace "{{PRD}}" prdRef |>.replace "{{PROGRESS}}" progressRef
  match extra? with
  | some extra =>
      if extra.trimAscii.toString == "" then
        pure prompt
      else
        pure s!"{extra}\n\n{prompt}"
  | none => pure prompt

end Ralph
