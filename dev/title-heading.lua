-- Pandoc Lua filter: if the document has a metadata `title`, render it as the
-- single level-1 heading and demote every existing heading one level so they
-- nest beneath it. No-op when there is no title.
--
-- Used (before strip-sections.lua) when generating README.md so the metadata
-- title is not lost.
function Pandoc(doc)
  local title = doc.meta.title
  if not title then
    return nil
  end

  local blocks = pandoc.walk_block(pandoc.Div(doc.blocks), {
    Header = function(h)
      h.level = math.min(h.level + 1, 6)
      return h
    end,
  }).content

  table.insert(blocks, 1, pandoc.Header(1, pandoc.utils.stringify(title)))
  return pandoc.Pandoc(blocks, doc.meta)
end
