import Std

namespace Ralph

/-- Load a prompt template and substitute PRD/progress references. -/
def loadPrompt (templatePath prdPath progressPath : System.FilePath) : IO String := do
  let template ← IO.FS.readFile templatePath
  let prdRef := s!"@{prdPath.toString}"
  let progressRef := s!"@{progressPath.toString}"
  let prompt := template.replace "{{PRD}}" prdRef |>.replace "{{PROGRESS}}" progressRef
  pure prompt

end Ralph
