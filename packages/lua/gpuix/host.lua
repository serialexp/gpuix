local host = {}

function host.text(value)
    return gpuix.text(value)
end

function host.set_text(handle, value)
    gpuix.set_text(handle, value)
end

function host.set_style(handle, style)
    gpuix.set_style(handle, style)
end

function host.set_prop(handle, name, value)
    gpuix.set_prop(handle, name, value)
end

function host.set_child(handle, child)
    gpuix.set_child(handle, child)
end

function host.set_children(handle, children)
    gpuix.set_children(handle, children)
end

function host.on_root_cleanup(callback)
    gpuix.on_root_cleanup(callback)
end

return host
