local ui = gpuix

local function reducer(state, action)
    if action.type == "select_conversation" then
        return {
            active_id = action.id,
            compact = state.compact,
            mode = state.mode,
            selected_activity = state.selected_activity,
            draft = state.draft,
        }
    end
    if action.type == "toggle_sidebar" then
        return {
            active_id = state.active_id,
            compact = not state.compact,
            mode = state.mode,
            selected_activity = state.selected_activity,
            draft = state.draft,
        }
    end
    if action.type == "set_mode" then
        return {
            active_id = state.active_id,
            compact = state.compact,
            mode = action.mode,
            selected_activity = state.selected_activity,
            draft = state.draft,
        }
    end
    if action.type == "select_activity" then
        return {
            active_id = state.active_id,
            compact = state.compact,
            mode = state.mode,
            selected_activity = action.id,
            draft = state.draft,
        }
    end
    if action.type == "set_draft" then
        return {
            active_id = state.active_id,
            compact = state.compact,
            mode = state.mode,
            selected_activity = state.selected_activity,
            draft = action.value,
        }
    end
    return state
end

return ui.create_store("workspace", reducer, {
    active_id = "runtime",
    compact = false,
    mode = "markdown",
    selected_activity = "activity-1",
    draft = "",
})
