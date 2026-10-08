local initial = {
    selected = "runtime",
    compact = false,
    show_activity = true,
    draft = "",
    messages = {
        { id = "runtime", title = "Fine-grained runtime", body = "Signals update bindings without rerendering components." },
        { id = "lists", title = "Keyed lists", body = "For keeps local state attached to an item; Index keeps it attached to a position." },
        { id = "windows", title = "Shared windows", body = "The inspector and workspace subscribe to the same store." },
    },
    activity = { "Workspace opened" },
    next_id = 1,
}

return gpuix.create_store("solid-workspace", function(state, action)
    local next_state = {}
    for key, value in pairs(state) do next_state[key] = value end
    if action.type == "select" then next_state.selected = action.id
    elseif action.type == "compact" then next_state.compact = not state.compact
    elseif action.type == "activity" then next_state.show_activity = not state.show_activity
    elseif action.type == "draft" then next_state.draft = action.value
    elseif action.type == "reverse" then
        next_state.messages = {}
        for index = #state.messages, 1, -1 do
            next_state.messages[#next_state.messages + 1] = state.messages[index]
        end
    elseif action.type == "send" then
        if state.draft == "" then return state end
        local id = "message-" .. state.next_id
        next_state.next_id = state.next_id + 1
        next_state.messages = {}
        for index, message in ipairs(state.messages) do next_state.messages[index] = message end
        next_state.messages[#next_state.messages + 1] = {
            id = id, title = state.draft, body = state.draft,
        }
        next_state.selected, next_state.draft = id, ""
        next_state.activity = {}
        for index, entry in ipairs(state.activity) do next_state.activity[index] = entry end
        next_state.activity[#next_state.activity + 1] = "Sent " .. id
    else return state end
    return next_state
end, initial)
