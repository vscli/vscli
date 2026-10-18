'use strict';
const { Position, Range, Uri } = require('./api-types.cjs');
const { SnippetString } = require('./snippet-string.cjs');
module.exports = require('./edit-types.cjs').createEditTypes({ Position, Range, Uri, SnippetString });
