export interface Column<T> {
  key: string;
  title: string;
  width?: string;
  align?: "left" | "right" | "center";
  sortable?: boolean;
  render?: (item: T) => string | number | undefined;
}

