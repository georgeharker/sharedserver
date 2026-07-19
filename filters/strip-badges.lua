-- Strip shields.io badges from the rendered docs site.
--
-- Badges are GitHub-README furniture and add nothing to the published site.
-- They also break the docs build: `embed-resources: true` makes Quarto fetch
-- and inline every external image at render time, so when shields.io rate-limits
-- (it now 403s the CI runner) pandoc emits `[WARNING] Could not fetch resource`
-- — and the docs workflow fails on any warning.
--
-- Doing it here rather than in the markdown keeps README.md clean, ordinary
-- markdown: GitHub still renders the badge normally, with no wrapper divs or
-- marker syntax leaking into the page.

local function is_badge(src)
  return src ~= nil and src:match("img%.shields%.io") ~= nil
end

-- A badge is usually a link wrapping the image ([![alt][ref]](target)).
-- Drop the whole link so no empty anchor is left behind.
function Link(el)
  if #el.content == 1
     and el.content[1].t == "Image"
     and is_badge(el.content[1].src) then
    return {}
  end
end

-- Bare (unlinked) badge images.
function Image(el)
  if is_badge(el.src) then
    return {}
  end
end

-- Removing the badge can leave an otherwise-empty paragraph; drop it so the
-- rendered page has no stray blank block where the badge row used to be.
function Para(el)
  if #el.content == 0 then
    return {}
  end
end
