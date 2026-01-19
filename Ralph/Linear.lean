import Std
import Lean.Data.Json
import Ralph.Options

namespace Ralph

open Lean

structure LinearProject where
  id : String
  name : String
  description : String
  url : String
  deriving Repr

structure LinearDoc where
  id : String
  title : String
  content : String
  url : String
  deriving Repr

structure LinearSync where
  project : LinearProject
  doc : LinearDoc
  deriving Repr

private def runGit (args : Array String) : IO (Option String) := do
  let out ← IO.Process.output { cmd := "git", args } none
  if out.exitCode == 0 then
    return some out.stdout.trimAscii.toString
  return none

private def readFile? (path : System.FilePath) : IO (Option String) := do
  if (← path.pathExists) then
    return some (← IO.FS.readFile path)
  return none

private def tokenFromEnv : IO (Option String) := do
  for name in ["LINEAR_API_KEY", "LINEAR_TOKEN", "LINEAR_API_TOKEN"] do
    if let some value ← IO.getEnv name then
      if value.trimAscii.toString != "" then
        return some value
  return none

private def tokenFromConfig (configPath : System.FilePath) : IO (Option String) := do
  if !(← configPath.pathExists) then
    return none
  let content ← IO.FS.readFile configPath
  let parts := content.splitOn "Bearer lin_api_"
  match parts with
  | _ :: rest :: _ =>
      let tokenTail := "lin_api_" ++ rest
      let chars := tokenTail.toList
      let valid := chars.takeWhile fun c => c.isAlphanum || c == '_' || c == '-'
      return some (String.ofList valid)
  | _ =>
      return none

private def linearToken : IO (Option String) := do
  if let some t ← tokenFromEnv then
    return some t
  let home ← IO.getEnv "HOME"
  let config :=
    match home with
    | some h => System.FilePath.mk h / ".codex" / "config.toml"
    | none => System.FilePath.mk ".codex" / "config.toml"
  tokenFromConfig config

private def jsonObj (pairs : List (String × Json)) : Json :=
  Json.mkObj pairs

private def graphql (token payload : String) : IO (Option Json) := do
  let args := #[
    "-sS",
    "-X", "POST",
    "-H", s!"Authorization: Bearer {token}",
    "-H", "Content-Type: application/json",
    "--data-binary", "@-",
    "https://api.linear.app/graphql"
  ]
  let out ← IO.Process.output { cmd := "curl", args } (some payload)
  if out.exitCode != 0 then
    return none
  match Json.parse out.stdout with
  | .ok j => return some j
  | .error _ => return none

private def getObjVal? (j : Json) (k : String) : Option Json :=
  (j.getObjVal? k).toOption

private def getStr? (j : Json) : Option String :=
  match j with
  | .str s => some s
  | _ => none

private def getArr? (j : Json) : Option (Array Json) :=
  match j with
  | .arr a => some a
  | _ => none

private def parseProjects (j : Json) : List LinearProject :=
  let data? := getObjVal? j "data"
  let proj? := data? >>= fun d => getObjVal? d "projects"
  let nodes? := proj? >>= fun p => getObjVal? p "nodes"
  match nodes? >>= getArr? with
  | none => []
  | some arr =>
      arr.toList.filterMap fun node =>
        let id? := getObjVal? node "id" >>= getStr?
        let name? := getObjVal? node "name" >>= getStr?
        let desc? := getObjVal? node "description" >>= getStr?
        let url? := getObjVal? node "url" >>= getStr?
        match id?, name?, desc?, url? with
        | some id, some name, some desc, some url =>
            some { id, name, description := desc, url }
        | _, _, _, _ => none

private def parseDocs (j : Json) : List LinearDoc :=
  let data? := getObjVal? j "data"
  let proj? := data? >>= fun d => getObjVal? d "project"
  let docs? := proj? >>= fun p => getObjVal? p "documents"
  let nodes? := docs? >>= fun p => getObjVal? p "nodes"
  match nodes? >>= getArr? with
  | none => []
  | some arr =>
      arr.toList.filterMap fun node =>
        let id? := getObjVal? node "id" >>= getStr?
        let title? := getObjVal? node "title" >>= getStr?
        let content? := getObjVal? node "content" >>= getStr?
        let url? := getObjVal? node "url" >>= getStr?
        match id?, title?, content?, url? with
        | some id, some title, some content, some url =>
            some { id, title, content, url }
        | _, _, _, _ => none

private def parseDoc (j : Json) : Option LinearDoc :=
  let data? := getObjVal? j "data"
  let doc? := data? >>= fun d => getObjVal? d "documentCreate"
  let node? := doc? >>= fun d => getObjVal? d "document"
  match node? with
  | none => none
  | some node =>
      let id? := getObjVal? node "id" >>= getStr?
      let title? := getObjVal? node "title" >>= getStr?
      let content? := getObjVal? node "content" >>= getStr?
      let url? := getObjVal? node "url" >>= getStr?
      match id?, title?, content?, url? with
      | some id, some title, some content, some url =>
          some { id, title, content, url }
      | _, _, _, _ => none

private def contains (haystack needle : String) : Bool :=
  haystack.toLower.toSlice.contains needle.toLower

private def projectScore (p : LinearProject) (repoName : String) (repoUrl? : Option String) : Nat :=
  let base := (if contains p.name repoName then 2 else 0) + (if contains p.description repoName then 1 else 0)
  match repoUrl? with
  | some url => if contains p.description url then base + 4 else base
  | none => base

private def findProject (projects : List LinearProject) (repoName : String) (repoUrl? : Option String) : Option LinearProject :=
  let (_bestScore, best) := projects.foldl
    (fun (acc : Nat × Option LinearProject) proj =>
      let score := projectScore proj repoName repoUrl?
      if score > acc.1 then
        (score, some proj)
      else
        acc)
    (0, none)
  best

private def queryProjects (token : String) : IO (List LinearProject) := do
  let query :=
    "query Projects($first: Int!) { projects(first: $first) { nodes { id name description url } } }"
  let variables := jsonObj [("first", Json.num (JsonNumber.fromNat 50))]
  let payload := jsonObj [("query", Json.str query), ("variables", variables)] |> Json.compress
  match (← graphql token payload) with
  | some j => return parseProjects j
  | none => return []

private def queryDocs (token : String) (projectId : String) : IO (List LinearDoc) := do
  let query :=
    "query ProjectDocs($id: String!) { project(id: $id) { documents(first: 50) { nodes { id title content url } } } }"
  let variables := jsonObj [("id", Json.str projectId)]
  let payload := jsonObj [("query", Json.str query), ("variables", variables)] |> Json.compress
  match (← graphql token payload) with
  | some j => return parseDocs j
  | none => return []

private def createDoc (token : String) (projectId title content : String) : IO (Option LinearDoc) := do
  let query :=
    "mutation CreateDoc($input: DocumentCreateInput!) { documentCreate(input: $input) { document { id title content url } } }"
  let input := jsonObj [("projectId", Json.str projectId), ("title", Json.str title), ("content", Json.str content)]
  let variables := jsonObj [("input", input)]
  let payload := jsonObj [("query", Json.str query), ("variables", variables)] |> Json.compress
  match (← graphql token payload) with
  | some j => return parseDoc j
  | none => return none

/-- Sync the Linear PRD doc into the local PRD path. Returns sync info if successful. -/
def syncLinearPRD (opts : Options) : IO (Option LinearSync) := do
  if opts.noLinear then
    return none
  let token? ← linearToken
  let some token := token? | return none
  let repoName := (← IO.currentDir).fileName.getD "repo"
  let repoUrl? ← runGit #["remote", "get-url", "origin"]
  let projects ← queryProjects token
  let some project := findProject projects repoName repoUrl? | return none
  let docs ← queryDocs token project.id
  let prdDoc? :=
    docs.find? fun doc => contains doc.title "prd"
  let prdDoc ←
    match prdDoc? with
    | some doc => pure doc
    | none =>
        let content ←
          match opts.prd with
          | some path => do
              let text? ← readFile? path
              pure (text?.getD "")
          | none => pure ""
        let title := s!"{project.name} PRD"
        match (← createDoc token project.id title content) with
        | some doc => pure doc
        | none => return none
  match opts.prd with
  | some path =>
      IO.FS.writeFile path prdDoc.content
  | none => pure ()
  return some { project, doc := prdDoc }

end Ralph
