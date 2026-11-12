--!strict
-- Written once from eval/tasks.json, not edited after running.

type Vec2 = { x: number, y: number }

function move_x(vx: number, input: number, dt: number): number
    if input ~= 0 then
        vx += 30 * input * dt
    else
        local f = 20 * dt
        if vx > 0 then
            vx = math.max(0, vx - f)
        elseif vx < 0 then
            vx = math.min(0, vx + f)
        end
    end
    return math.clamp(vx, -6, 6)
end

function patrol(points: { Vec2 }, index: number, pos: Vec2, speed: number, dt: number)
    local target = points[index]
    local dx, dy = target.x - pos.x, target.y - pos.y
    local dist = math.sqrt(dx * dx + dy * dy)
    local step = speed * dt
    if dist <= step then
        local nextIndex = index + 1
        if nextIndex > #points then
            nextIndex = 1
        end
        return { index = nextIndex, x = target.x, y = target.y }
    end
    return { index = index, x = pos.x + dx / dist * step, y = pos.y + dy / dist * step }
end

type Slot = { item: string, count: number }

function add_item(slots: { Slot }, item: string, count: number, max_stack: number): number
    for _, slot in slots do
        if count <= 0 then
            break
        end
        if slot.item == item and slot.count < max_stack then
            local add = math.min(count, max_stack - slot.count)
            slot.count += add
            count -= add
        end
    end
    for _, slot in slots do
        if count <= 0 then
            break
        end
        if slot.item == "" and slot.count == 0 then
            local add = math.min(count, max_stack)
            slot.item = item
            slot.count = add
            count -= add
        end
    end
    return count
end

type Mark = { at: number, name: string }

function timer_events(before: number, after: number, marks: { Mark }): { string }
    local crossed: { { mark: Mark, order: number } } = {}
    for i, m in marks do
        if m.at > after and m.at <= before then
            table.insert(crossed, { mark = m, order = i })
        end
    end
    table.sort(crossed, function(a, b)
        if a.mark.at ~= b.mark.at then
            return a.mark.at > b.mark.at
        end
        return a.order < b.order
    end)
    local names = {}
    for _, c in crossed do
        table.insert(names, c.mark.name)
    end
    return names
end

type Other = { id: number, x: number, y: number, alive: boolean }

function nearest(from: Vec2, others: { Other }): number?
    local bestId: number? = nil
    local bestD = math.huge
    for _, o in others do
        if o.alive then
            local dx, dy = o.x - from.x, o.y - from.y
            local d = dx * dx + dy * dy
            if d < bestD or (d == bestD and bestId ~= nil and o.id < bestId) then
                bestD = d
                bestId = o.id
            end
        end
    end
    return bestId
end

type LootEntry = { name: string, weight: number }

function loot(entries: { LootEntry }, r: number): string
    local total = 0
    for _, e in entries do
        total += e.weight
    end
    if total == 0 then
        return ""
    end
    local target = r * total
    local sum = 0
    for _, e in entries do
        sum += e.weight
        if e.weight > 0 and target < sum then
            return e.name
        end
    end
    return ""
end

function path_len(grid: { string }): number
    local h = #grid
    local w = #grid[1]
    local sx, sy = 0, 0
    for y = 1, h do
        for x = 1, w do
            if string.sub(grid[y], x, x) == "S" then
                sx, sy = x, y
            end
        end
    end
    local dist: { [number]: number } = {}
    local function key(x: number, y: number): number
        return (y - 1) * w + x
    end
    local queue = { { sx, sy } }
    dist[key(sx, sy)] = 0
    local head = 1
    while head <= #queue do
        local cur = queue[head]
        head += 1
        local cx, cy = cur[1], cur[2]
        local d = dist[key(cx, cy)]
        if string.sub(grid[cy], cx, cx) == "G" then
            return d
        end
        for _, dir in { { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } } do
            local nx, ny = cx + dir[1], cy + dir[2]
            if nx >= 1 and nx <= w and ny >= 1 and ny <= h then
                local c = string.sub(grid[ny], nx, nx)
                if c ~= "#" and dist[key(nx, ny)] == nil then
                    dist[key(nx, ny)] = d + 1
                    table.insert(queue, { nx, ny })
                end
            end
        end
    end
    return -1
end

function score_text(score: number, combo: number): string
    local s = string.format("Score: %05d", math.min(score, 99999))
    if combo > 1 then
        s ..= " x" .. tostring(combo)
    end
    return s
end

type ScoreEntry = { name: string, score: number }

function top_scores(entries: { ScoreEntry }, n: number): { string }
    local sorted = table.clone(entries)
    table.sort(sorted, function(a, b)
        if a.score ~= b.score then
            return a.score > b.score
        end
        return a.name < b.name
    end)
    local out = {}
    for i = 1, math.min(n, #sorted) do
        table.insert(out, sorted[i].name)
    end
    return out
end

function parse_config(text: string): { [string]: any }
    local result = {}
    for _, segment in string.split(text, ";") do
        local eq = string.find(segment, "=", 1, true)
        if eq then
            local key = string.gsub(string.sub(segment, 1, eq - 1), "^%s*(.-)%s*$", "%1")
            local value = string.gsub(string.sub(segment, eq + 1), "^%s*(.-)%s*$", "%1")
            if key ~= "" then
                if value == "true" then
                    result[key] = true
                elseif value == "false" then
                    result[key] = false
                elseif string.match(value, "^%-?%d+%.?%d*$") then
                    result[key] = tonumber(value)
                else
                    result[key] = value
                end
            end
        end
    end
    return result
end
