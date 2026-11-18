// The tools of the open Excel workbook, through Office.js's Excel API. `excel` is Office.js's `Excel` object, passed in so a test can
// give a stand-in. Reads answer at once; writing a range is asked in the pane first (`write: true`), by `tools.js`.

/** The most cells one answer carries. */
const MAX_CELLS = 50_000;

/** `Sheet1!A1:B2` or `'My sheet'!A1` as a sheet name (or none) and an address. */
export function splitAddress(address) {
  if (typeof address !== 'string' || address.trim() === '') throw new Error('Give an address such as Sheet1!A1:C5.');
  const text = address.trim();
  const bang = text.lastIndexOf('!');
  if (bang < 0) return { sheet: undefined, cells: text };
  const sheet = text.slice(0, bang).replace(/^'(.*)'$/, '$1').replace(/''/g, "'");
  return { sheet, cells: text.slice(bang + 1) };
}

/** `values` cut to `MAX_CELLS` cells, whole rows only, and whether it was cut. */
function limited(values) {
  const width = Math.max(1, values[0]?.length ?? 1);
  const rows = Math.max(1, Math.floor(MAX_CELLS / width));
  return { values: values.slice(0, rows), truncated: values.length > rows };
}

/** @param {typeof Excel} excel */
export function excelTools(excel) {
  const rangeOf = (context, address) => {
    const { sheet, cells } = splitAddress(address);
    const worksheet = sheet === undefined ? context.workbook.worksheets.getActiveWorksheet() : context.workbook.worksheets.getItem(sheet);
    return worksheet.getRange(cells);
  };

  return [
    {
      name: 'excel_get_selection',
      description: 'The cells the person has selected in the open workbook: their address, values and formulas.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      run: () =>
        excel.run(async (context) => {
          const range = context.workbook.getSelectedRange();
          range.load('address,values,formulas,rowCount,columnCount');
          await context.sync();
          const cut = limited(range.values);
          return {
            address: range.address,
            rows: range.rowCount,
            columns: range.columnCount,
            values: cut.values,
            formulas: range.formulas.slice(0, cut.values.length),
            truncated: cut.truncated,
          };
        }),
    },
    {
      name: 'excel_list_tables',
      description: 'The tables of the open workbook, with the sheet and range each is on.',
      inputSchema: { type: 'object', properties: {}, additionalProperties: false },
      run: () =>
        excel.run(async (context) => {
          const tables = context.workbook.tables;
          tables.load('items/name');
          await context.sync();
          const ranges = tables.items.map((table) => {
            const range = table.getRange();
            range.load('address,rowCount,columnCount');
            return range;
          });
          await context.sync();
          return { tables: tables.items.map((table, at) => ({ name: table.name, address: ranges[at].address, rows: ranges[at].rowCount, columns: ranges[at].columnCount })) };
        }),
    },
    {
      name: 'excel_get_table',
      description: 'The header and rows of one table of the open workbook.',
      inputSchema: {
        type: 'object',
        properties: { name: { type: 'string', description: 'The table\'s name from excel_list_tables' } },
        required: ['name'],
        additionalProperties: false,
      },
      run: ({ name }) => {
        if (typeof name !== 'string' || name === '') throw new Error('Name the table.');
        return excel.run(async (context) => {
          const range = context.workbook.tables.getItem(name).getRange();
          range.load('address,values');
          await context.sync();
          const cut = limited(range.values);
          return { name, address: range.address, header: cut.values[0], rows: cut.values.slice(1), truncated: cut.truncated };
        });
      },
    },
    {
      name: 'excel_read_range',
      description: 'The values and formulas of a range of the open workbook, such as Sheet1!A1:C20 (the active sheet when no sheet is named).',
      inputSchema: {
        type: 'object',
        properties: { address: { type: 'string' } },
        required: ['address'],
        additionalProperties: false,
      },
      run: ({ address }) =>
        excel.run(async (context) => {
          const range = rangeOf(context, address);
          range.load('address,values,formulas');
          await context.sync();
          const cut = limited(range.values);
          return { address: range.address, values: cut.values, formulas: range.formulas.slice(0, cut.values.length), truncated: cut.truncated };
        }),
    },
    {
      name: 'excel_write_range',
      description: 'Writes values (or formulas starting with =) into a range of the open workbook. The values are rows of cells and must fit the range.',
      inputSchema: {
        type: 'object',
        properties: {
          address: { type: 'string', description: 'Such as Sheet1!A1:B2; the active sheet when no sheet is named' },
          values: { type: 'array', items: { type: 'array', items: {} }, description: 'Rows of cell values' },
        },
        required: ['address', 'values'],
        additionalProperties: false,
      },
      write: true,
      summary: ({ address, values }) => {
        const rows = Array.isArray(values) ? values.length : 0;
        const columns = Array.isArray(values?.[0]) ? values[0].length : 0;
        return `Write ${rows} row${rows === 1 ? '' : 's'} of ${columns} to ${String(address ?? '')}`;
      },
      run: ({ address, values }) => {
        if (!Array.isArray(values) || values.length === 0 || !values.every((row) => Array.isArray(row) && row.length === values[0].length)) {
          throw new Error('values is rows of cells, every row the same length.');
        }
        return excel.run(async (context) => {
          const range = rangeOf(context, address);
          range.load('rowCount,columnCount,address');
          await context.sync();
          if (range.rowCount !== values.length || range.columnCount !== values[0].length) {
            throw new Error(`${range.address} is ${range.rowCount} by ${range.columnCount} and the values are ${values.length} by ${values[0].length}.`);
          }
          range.values = values;
          await context.sync();
          return { written: range.address, cells: values.length * values[0].length };
        });
      },
    },
  ];
}
