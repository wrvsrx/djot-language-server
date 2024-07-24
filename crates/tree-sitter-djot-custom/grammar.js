module.exports = grammar({
  name: "djot",

  extras: ($) => [$._ignored],

  rules: {
    document: ($) => repeat($.block),

    block: ($) => choice($.section, $.heading, $.paragraph, $.blankline),

    section: ($) => seq($._section_start, $.heading, repeat($.block), $._section_end),
    heading: ($) => seq($._heading_start, $.heading_marker, repeat(choice($.heading_marker, $.inline)), $._heading_end),
    paragraph: ($) => seq($._paragraph_start, repeat1($.inline), $._paragraph_end),
    blankline: ($) => seq($._blankline_start, $._blankline_end),

    inline: ($) => choice($.str, $.softbreak),
  },

  externals: ($) => [
    $._section_start,
    $._section_end,

    $._heading_start,
    $.heading_marker,
    $._heading_end,

    $._paragraph_start,
    $._paragraph_end,

    $._blankline_start,
    $._blankline_end,

    $.str,
    $.softbreak,

    $._ignored,
  ],
});
