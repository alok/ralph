namespace Ralph

/-- Compile-time loop phases. -/
inductive Phase where
  | init
  | running
  | done
  deriving Repr, DecidableEq

/-- Loop state indexed by phase to enforce legal transitions. -/
structure LoopState (phase : Phase) where
  iter : Nat
  max : Nat
  deriving Repr

namespace LoopState

/-- Start in the init phase. -/
def start (max : Nat) : LoopState .init :=
  { iter := 0, max }

/-- Transition from init to running. -/
def begin (st : LoopState .init) : LoopState .running :=
  { iter := st.iter, max := st.max }

/-- Transition from running to done. -/
def finish (st : LoopState .running) : LoopState .done :=
  { iter := st.iter, max := st.max }

/-- Advance one iteration while staying in the running phase. -/
def step (st : LoopState .running) : LoopState .running :=
  { iter := st.iter + 1, max := st.max }

/-- True if another iteration should run. -/
def shouldContinue (st : LoopState .running) : Bool :=
  st.iter < st.max

end LoopState

end Ralph
