const {addTable} = require('../utils');
const ImportManager = require('../importer');

function migrate() {
    // `module.exports = { ...require('./schema') }`: schema's names.
    addTable('posts');
    // `module.exports = require('./import-manager')`: that module's
    // instance, so its class's method.
    return ImportManager.getExtensions();
}
module.exports = { migrate };
