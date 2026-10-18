"use strict";
/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Microsoft Corporation. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/
Object.defineProperty(exports, "__esModule", { value: true });
exports.Scanner = exports.TokenType = void 0;
const charCode_js_1 = require("../../../base/common/charCode.js");
const errors_js_1 = require("../../../base/common/errors.js");
const nls_js_1 = require("../../../nls.js");
var TokenType;
(function (TokenType) {
    TokenType[TokenType["LParen"] = 0] = "LParen";
    TokenType[TokenType["RParen"] = 1] = "RParen";
    TokenType[TokenType["Neg"] = 2] = "Neg";
    TokenType[TokenType["Eq"] = 3] = "Eq";
    TokenType[TokenType["NotEq"] = 4] = "NotEq";
    TokenType[TokenType["Lt"] = 5] = "Lt";
    TokenType[TokenType["LtEq"] = 6] = "LtEq";
    TokenType[TokenType["Gt"] = 7] = "Gt";
    TokenType[TokenType["GtEq"] = 8] = "GtEq";
    TokenType[TokenType["RegexOp"] = 9] = "RegexOp";
    TokenType[TokenType["RegexStr"] = 10] = "RegexStr";
    TokenType[TokenType["True"] = 11] = "True";
    TokenType[TokenType["False"] = 12] = "False";
    TokenType[TokenType["In"] = 13] = "In";
    TokenType[TokenType["Not"] = 14] = "Not";
    TokenType[TokenType["And"] = 15] = "And";
    TokenType[TokenType["Or"] = 16] = "Or";
    TokenType[TokenType["Str"] = 17] = "Str";
    TokenType[TokenType["QuotedStr"] = 18] = "QuotedStr";
    TokenType[TokenType["Error"] = 19] = "Error";
    TokenType[TokenType["EOF"] = 20] = "EOF";
})(TokenType || (exports.TokenType = TokenType = {}));
function hintDidYouMean(...meant) {
    switch (meant.length) {
        case 1:
            return (0, nls_js_1.localize)('contextkey.scanner.hint.didYouMean1', "Did you mean {0}?", meant[0]);
        case 2:
            return (0, nls_js_1.localize)('contextkey.scanner.hint.didYouMean2', "Did you mean {0} or {1}?", meant[0], meant[1]);
        case 3:
            return (0, nls_js_1.localize)('contextkey.scanner.hint.didYouMean3', "Did you mean {0}, {1} or {2}?", meant[0], meant[1], meant[2]);
        default: // we just don't expect that many
            return undefined;
    }
}
const hintDidYouForgetToOpenOrCloseQuote = (0, nls_js_1.localize)('contextkey.scanner.hint.didYouForgetToOpenOrCloseQuote', "Did you forget to open or close the quote?");
const hintDidYouForgetToEscapeSlash = (0, nls_js_1.localize)('contextkey.scanner.hint.didYouForgetToEscapeSlash', "Did you forget to escape the '/' (slash) character? Put two backslashes before it to escape, e.g., '\\\\/\'.");
/**
 * A simple scanner for context keys.
 *
 * Example:
 *
 * ```ts
 * const scanner = new Scanner().reset('resourceFileName =~ /docker/ && !config.docker.enabled');
 * const tokens = [...scanner];
 * if (scanner.errorTokens.length > 0) {
 *     scanner.errorTokens.forEach(err => console.error(`Unexpected token at ${err.offset}: ${err.lexeme}\nHint: ${err.additional}`));
 * } else {
 *     // process tokens
 * }
 * ```
 */
class Scanner {
    static getLexeme(token) {
        switch (token.type) {
            case TokenType.LParen:
                return '(';
            case TokenType.RParen:
                return ')';
            case TokenType.Neg:
                return '!';
            case TokenType.Eq:
                return token.isTripleEq ? '===' : '==';
            case TokenType.NotEq:
                return token.isTripleEq ? '!==' : '!=';
            case TokenType.Lt:
                return '<';
            case TokenType.LtEq:
                return '<=';
            case TokenType.Gt:
                return '>';
            case TokenType.GtEq:
                return '>=';
            case TokenType.RegexOp:
                return '=~';
            case TokenType.RegexStr:
                return token.lexeme;
            case TokenType.True:
                return 'true';
            case TokenType.False:
                return 'false';
            case TokenType.In:
                return 'in';
            case TokenType.Not:
                return 'not';
            case TokenType.And:
                return '&&';
            case TokenType.Or:
                return '||';
            case TokenType.Str:
                return token.lexeme;
            case TokenType.QuotedStr:
                return token.lexeme;
            case TokenType.Error:
                return token.lexeme;
            case TokenType.EOF:
                return 'EOF';
            default:
                throw (0, errors_js_1.illegalState)(`unhandled token type: ${JSON.stringify(token)}; have you forgotten to add a case?`);
        }
    }
    static _regexFlags = new Set(['i', 'g', 's', 'm', 'y', 'u'].map(ch => ch.charCodeAt(0)));
    static _keywords = new Map([
        ['not', TokenType.Not],
        ['in', TokenType.In],
        ['false', TokenType.False],
        ['true', TokenType.True],
    ]);
    _input = '';
    _start = 0;
    _current = 0;
    _tokens = [];
    _errors = [];
    get errors() {
        return this._errors;
    }
    reset(value) {
        this._input = value;
        this._start = 0;
        this._current = 0;
        this._tokens = [];
        this._errors = [];
        return this;
    }
    scan() {
        while (!this._isAtEnd()) {
            this._start = this._current;
            const ch = this._advance();
            switch (ch) {
                case charCode_js_1.CharCode.OpenParen:
                    this._addToken(TokenType.LParen);
                    break;
                case charCode_js_1.CharCode.CloseParen:
                    this._addToken(TokenType.RParen);
                    break;
                case charCode_js_1.CharCode.ExclamationMark:
                    if (this._match(charCode_js_1.CharCode.Equals)) {
                        const isTripleEq = this._match(charCode_js_1.CharCode.Equals); // eat last `=` if `!==`
                        this._tokens.push({ type: TokenType.NotEq, offset: this._start, isTripleEq });
                    }
                    else {
                        this._addToken(TokenType.Neg);
                    }
                    break;
                case charCode_js_1.CharCode.SingleQuote:
                    this._quotedString();
                    break;
                case charCode_js_1.CharCode.Slash:
                    this._regex();
                    break;
                case charCode_js_1.CharCode.Equals:
                    if (this._match(charCode_js_1.CharCode.Equals)) { // support `==`
                        const isTripleEq = this._match(charCode_js_1.CharCode.Equals); // eat last `=` if `===`
                        this._tokens.push({ type: TokenType.Eq, offset: this._start, isTripleEq });
                    }
                    else if (this._match(charCode_js_1.CharCode.Tilde)) {
                        this._addToken(TokenType.RegexOp);
                    }
                    else {
                        this._error(hintDidYouMean('==', '=~'));
                    }
                    break;
                case charCode_js_1.CharCode.LessThan:
                    this._addToken(this._match(charCode_js_1.CharCode.Equals) ? TokenType.LtEq : TokenType.Lt);
                    break;
                case charCode_js_1.CharCode.GreaterThan:
                    this._addToken(this._match(charCode_js_1.CharCode.Equals) ? TokenType.GtEq : TokenType.Gt);
                    break;
                case charCode_js_1.CharCode.Ampersand:
                    if (this._match(charCode_js_1.CharCode.Ampersand)) {
                        this._addToken(TokenType.And);
                    }
                    else {
                        this._error(hintDidYouMean('&&'));
                    }
                    break;
                case charCode_js_1.CharCode.Pipe:
                    if (this._match(charCode_js_1.CharCode.Pipe)) {
                        this._addToken(TokenType.Or);
                    }
                    else {
                        this._error(hintDidYouMean('||'));
                    }
                    break;
                // TODO@ulugbekna: 1) rewrite using a regex 2) reconsider what characters are considered whitespace, including unicode, nbsp, etc.
                case charCode_js_1.CharCode.Space:
                case charCode_js_1.CharCode.CarriageReturn:
                case charCode_js_1.CharCode.Tab:
                case charCode_js_1.CharCode.LineFeed:
                case charCode_js_1.CharCode.NoBreakSpace: // &nbsp
                    break;
                default:
                    this._string();
            }
        }
        this._start = this._current;
        this._addToken(TokenType.EOF);
        return Array.from(this._tokens);
    }
    _match(expected) {
        if (this._isAtEnd()) {
            return false;
        }
        if (this._input.charCodeAt(this._current) !== expected) {
            return false;
        }
        this._current++;
        return true;
    }
    _advance() {
        return this._input.charCodeAt(this._current++);
    }
    _peek() {
        return this._isAtEnd() ? charCode_js_1.CharCode.Null : this._input.charCodeAt(this._current);
    }
    _addToken(type) {
        this._tokens.push({ type, offset: this._start });
    }
    _error(additional) {
        const offset = this._start;
        const lexeme = this._input.substring(this._start, this._current);
        const errToken = { type: TokenType.Error, offset: this._start, lexeme };
        this._errors.push({ offset, lexeme, additionalInfo: additional });
        this._tokens.push(errToken);
    }
    // u - unicode, y - sticky // TODO@ulugbekna: we accept double quotes as part of the string rather than as a delimiter (to preserve old parser's behavior)
    stringRe = /[a-zA-Z0-9_<>\-\./\\:\*\?\+\[\]\^,#@;"%\$\p{L}-]+/uy;
    _string() {
        this.stringRe.lastIndex = this._start;
        const match = this.stringRe.exec(this._input);
        if (match) {
            this._current = this._start + match[0].length;
            const lexeme = this._input.substring(this._start, this._current);
            const keyword = Scanner._keywords.get(lexeme);
            if (keyword) {
                this._addToken(keyword);
            }
            else {
                this._tokens.push({ type: TokenType.Str, lexeme, offset: this._start });
            }
        }
    }
    // captures the lexeme without the leading and trailing '
    _quotedString() {
        while (this._peek() !== charCode_js_1.CharCode.SingleQuote && !this._isAtEnd()) { // TODO@ulugbekna: add support for escaping ' ?
            this._advance();
        }
        if (this._isAtEnd()) {
            this._error(hintDidYouForgetToOpenOrCloseQuote);
            return;
        }
        // consume the closing '
        this._advance();
        this._tokens.push({ type: TokenType.QuotedStr, lexeme: this._input.substring(this._start + 1, this._current - 1), offset: this._start + 1 });
    }
    /*
     * Lexing a regex expression: /.../[igsmyu]*
     * Based on https://github.com/microsoft/TypeScript/blob/9247ef115e617805983740ba795d7a8164babf89/src/compiler/scanner.ts#L2129-L2181
     *
     * Note that we want slashes within a regex to be escaped, e.g., /file:\\/\\/\\// should match `file:///`
     */
    _regex() {
        let p = this._current;
        let inEscape = false;
        let inCharacterClass = false;
        while (true) {
            if (p >= this._input.length) {
                this._current = p;
                this._error(hintDidYouForgetToEscapeSlash);
                return;
            }
            const ch = this._input.charCodeAt(p);
            if (inEscape) { // parsing an escape character
                inEscape = false;
            }
            else if (ch === charCode_js_1.CharCode.Slash && !inCharacterClass) { // end of regex
                p++;
                break;
            }
            else if (ch === charCode_js_1.CharCode.OpenSquareBracket) {
                inCharacterClass = true;
            }
            else if (ch === charCode_js_1.CharCode.Backslash) {
                inEscape = true;
            }
            else if (ch === charCode_js_1.CharCode.CloseSquareBracket) {
                inCharacterClass = false;
            }
            p++;
        }
        // Consume flags // TODO@ulugbekna: use regex instead
        while (p < this._input.length && Scanner._regexFlags.has(this._input.charCodeAt(p))) {
            p++;
        }
        this._current = p;
        const lexeme = this._input.substring(this._start, this._current);
        this._tokens.push({ type: TokenType.RegexStr, lexeme, offset: this._start });
    }
    _isAtEnd() {
        return this._current >= this._input.length;
    }
}
exports.Scanner = Scanner;
