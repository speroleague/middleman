module Model exposing (Model(..), count, next)


type Model
    = Initial State


type alias State =
    { count : Int
    , messages : List String
    }


count : Model -> Int
count (Initial state) =
    state.count


next : Model -> Model
next (Initial state) =
    Initial { count = state.count + 1, messages = state.messages }
