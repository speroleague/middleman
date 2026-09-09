module Main exposing (Model, init, main, update)

import Browser exposing (Program)
import Model exposing (Model(..), next)
import Update exposing (Msg(..), updateModel)


main : Program Flags Model
main =
    Browser.sandbox
        { init = init
        , update = update
        , view = view
        }
        |> Program.withFlags (Decode.decodeFlags { count = 0, messages = [] })


type alias Flags =
    { count : Int, messages : List String }


init : Flags -> Model
init flags =
    Initial { count = flags.count, messages = flags.messages }


update : Msg -> Model -> Model
update msg model =
    updateModel msg model


view : Model -> Html Msg
view model =
    Html.div [] [ Html.text ("count: " ++ String.fromInt (Model.count model)) ]
