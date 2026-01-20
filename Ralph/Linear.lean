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

structure LinearCache where
  repoName : String
  repoUrl : String
  project : LinearProject
  doc : LinearDoc
  deriving Repr

private def runGit (args : Array String) (cwd? : Option System.FilePath := none) : IO (Option String) := do
  let out ← IO.Process.output { cmd := "git", args, cwd := cwd? } none
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

private def authHeader (token : String) : String :=
  let t :=
    if token.startsWith "Bearer " then
      token.drop 7
    else
      token
  if t.startsWith "lin_api_" then
    s!"Authorization: {t}"
  else
    s!"Authorization: Bearer {t}"

private def graphql (token payload : String) : IO (Option Json) := do
  let args := #[
    "-sS",
    "-X", "POST",
    "-H", authHeader token,
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

private def graphqlWithRetries (token payload : String) (retries : Nat := 2) : IO (Option Json) := do
  let rec loop : Nat -> Nat -> IO (Option Json)
    | 0, _ => graphql token payload
    | n + 1, delaySec => do
        let res ← graphql token payload
        match res with
        | some j => return some j
        | none =>
            IO.sleep (UInt32.ofNat delaySec)
            loop n (delaySec * 2)
  loop retries 1

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

private def parseDocNode (node : Json) : Option LinearDoc :=
  let id? := getObjVal? node "id" >>= getStr?
  let title? := getObjVal? node "title" >>= getStr?
  let content? := getObjVal? node "content" >>= getStr?
  let url? := getObjVal? node "url" >>= getStr?
  match id?, title?, content?, url? with
  | some id, some title, some content, some url =>
      some { id, title, content, url }
  | _, _, _, _ => none

private def parseDocs (j : Json) : List LinearDoc :=
  let data? := getObjVal? j "data"
  let proj? := data? >>= fun d => getObjVal? d "project"
  let docs? := proj? >>= fun p => getObjVal? p "documents"
  let nodes? := docs? >>= fun p => getObjVal? p "nodes"
  match nodes? >>= getArr? with
  | none => []
  | some arr => arr.toList.filterMap parseDocNode

private def parseDoc (j : Json) : Option LinearDoc :=
  let data? := getObjVal? j "data"
  let doc? := data? >>= fun d => getObjVal? d "documentCreate"
  let node? := doc? >>= fun d => getObjVal? d "document"
  node? >>= parseDocNode

private def parseDocById (j : Json) : Option LinearDoc :=
  let data? := getObjVal? j "data"
  let node? := data? >>= fun d => getObjVal? d "document"
  node? >>= parseDocNode

private def contains (haystack needle : String) : Bool :=
  haystack.toLower.toSlice.contains needle.toLower

private def cachePath : IO System.FilePath := do
  let xdg? ← IO.getEnv "XDG_CACHE_HOME"
  let home? ← IO.getEnv "HOME"
  let base :=
    match xdg? with
    | some path => System.FilePath.mk path
    | none =>
        match home? with
        | some home => System.FilePath.mk home / ".cache"
        | none => System.FilePath.mk ".cache"
  return base / "ralph" / "linear_prd.json"

private def cacheToJson (cache : LinearCache) : Json :=
  jsonObj [
    ("repoName", Json.str cache.repoName),
    ("repoUrl", Json.str cache.repoUrl),
    ("project", jsonObj [
      ("id", Json.str cache.project.id),
      ("name", Json.str cache.project.name),
      ("description", Json.str cache.project.description),
      ("url", Json.str cache.project.url)
    ]),
    ("doc", jsonObj [
      ("id", Json.str cache.doc.id),
      ("title", Json.str cache.doc.title),
      ("content", Json.str cache.doc.content),
      ("url", Json.str cache.doc.url)
    ])
  ]

private def parseCache (j : Json) : Option LinearCache := do
  let repoName ← getObjVal? j "repoName" >>= getStr?
  let repoUrl := (getObjVal? j "repoUrl" >>= getStr?).getD ""
  let proj ← getObjVal? j "project"
  let projId ← getObjVal? proj "id" >>= getStr?
  let projName ← getObjVal? proj "name" >>= getStr?
  let projDesc ← getObjVal? proj "description" >>= getStr?
  let projUrl ← getObjVal? proj "url" >>= getStr?
  let doc ← getObjVal? j "doc"
  let docId ← getObjVal? doc "id" >>= getStr?
  let docTitle ← getObjVal? doc "title" >>= getStr?
  let docContent ← getObjVal? doc "content" >>= getStr?
  let docUrl ← getObjVal? doc "url" >>= getStr?
  return {
    repoName,
    repoUrl,
    project := { id := projId, name := projName, description := projDesc, url := projUrl },
    doc := { id := docId, title := docTitle, content := docContent, url := docUrl }
  }

private def cacheMatches (cache : LinearCache) (repoName : String) (repoUrl? : Option String) : Bool :=
  if cache.repoName == repoName then
    true
  else
    match repoUrl? with
    | some url =>
        cache.repoUrl != "" && (contains cache.repoUrl url || contains url cache.repoUrl)
    | none => false

private def readCache (repoName : String) (repoUrl? : Option String) : IO (Option LinearCache) := do
  let path ← cachePath
  if !(← path.pathExists) then
    return none
  let content ← IO.FS.readFile path
  match Json.parse content with
  | .error _ => return none
  | .ok j =>
      let cache? := parseCache j
      match cache? with
      | some cache =>
          if cacheMatches cache repoName repoUrl? then
            return some cache
          return none
      | none => return none

private def writeCache (cache : LinearCache) : IO Unit := do
  let path ← cachePath
  if let some parent := path.parent then
    IO.FS.createDirAll parent
  IO.FS.writeFile path (Json.compress (cacheToJson cache))

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
  match (← graphqlWithRetries token payload) with
  | some j => return parseProjects j
  | none => return []

private def queryDocs (token : String) (projectId : String) : IO (List LinearDoc) := do
  let query :=
    "query ProjectDocs($id: String!) { project(id: $id) { documents(first: 50) { nodes { id title content url } } } }"
  let variables := jsonObj [("id", Json.str projectId)]
  let payload := jsonObj [("query", Json.str query), ("variables", variables)] |> Json.compress
  match (← graphqlWithRetries token payload) with
  | some j => return parseDocs j
  | none => return []

private def queryDocById (token : String) (docId : String) : IO (Option LinearDoc) := do
  let query :=
    "query Document($id: String!) { document(id: $id) { id title content url } }"
  let variables := jsonObj [("id", Json.str docId)]
  let payload := jsonObj [("query", Json.str query), ("variables", variables)] |> Json.compress
  match (← graphqlWithRetries token payload) with
  | some j => return parseDocById j
  | none => return none

private def createDoc (token : String) (projectId title content : String) : IO (Option LinearDoc) := do
  let query :=
    "mutation CreateDoc($input: DocumentCreateInput!) { documentCreate(input: $input) { document { id title content url } } }"
  let input := jsonObj [("projectId", Json.str projectId), ("title", Json.str title), ("content", Json.str content)]
  let variables := jsonObj [("input", input)]
  let payload := jsonObj [("query", Json.str query), ("variables", variables)] |> Json.compress
  match (← graphqlWithRetries token payload) with
  | some j => return parseDoc j
  | none => return none

/-- Sync the Linear PRD doc into the local PRD path. Returns sync info if successful. -/
def syncLinearPRD (opts : Options) : IO (Option LinearSync) := do
  if opts.noLinear then
    return none
  let token? ← linearToken
  let some token := token? | return none
  let repoRoot? ← runGit #["rev-parse", "--show-toplevel"]
  let repoRootPath := repoRoot?.map System.FilePath.mk
  let cwd ← IO.currentDir
  let repoName :=
    match repoRootPath with
    | some root => root.fileName.getD "repo"
    | none => cwd.fileName.getD "repo"
  let repoUrl? ← runGit #["remote", "get-url", "origin"] repoRootPath
  if let some cache ← readCache repoName repoUrl? then
    let doc? ← queryDocById token cache.doc.id
    let doc := doc?.getD cache.doc
    let shouldUseCache := doc?.isSome || doc.content != ""
    if shouldUseCache then
      match opts.prd with
      | some path => IO.FS.writeFile path doc.content
      | none => pure ()
      writeCache { cache with doc := doc }
      return some { project := cache.project, doc := doc }
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
  writeCache { repoName, repoUrl := repoUrl?.getD "", project, doc := prdDoc }
  return some { project, doc := prdDoc }

end Ralph
