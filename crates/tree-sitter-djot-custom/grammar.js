module.exports = grammar({
  name: "djot",

  extras: (_) => ["\r"],

  rules: {
    document: ($) => repeat($.block),

    block: ($) => choice($.paragraph, $.blankline),

    paragraph: ($) => seq(repeat1($.inline), $.paragraph_end),
    inline: ($) => choice($.str, $.softbreak),
    str: ($) => /.+/,
    paragraph_end: ($) => /\n/,
  },

  externals: ($) => [
    $.blankline,
    $.softbreak,
    $.paragraph_end,
  ],
});
