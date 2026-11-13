using Unity.Burst;
using Unity.Collections;
using Unity.Jobs;
using Unity.Mathematics;

namespace Sample.Flow
{
    public enum CellKind { Source = 0, Pipe = 1, Sink = 2 }

    /// A cell of a flow network: flat data for the job.
    public struct Cell
    {
        public const int Ports = 2;
        public CellKind kind;
        public int amount, capacity;
        public int next;
    }

    [BurstCompile]
    struct FlowJob : IJob
    {
        public NativeArray<Cell> cells;
        [ReadOnly] public NativeArray<int> order;
        public NativeReference<long> tick;
        public int rate;

        public void Execute()
        {
            tick.Value = tick.Value + 1;
            for (var k = 0; k < order.Length; k++)
            {
                var i = order[k];
                var c = cells[i];
                if (c.kind == CellKind.Source) c.amount = math.min(c.capacity, c.amount + rate);
                if (c.next >= 0)
                {
                    var n = cells[c.next];
                    var moved = math.min(c.amount, n.capacity - n.amount);
                    n.amount += moved;
                    c.amount -= moved;
                    cells[c.next] = n;
                }
                cells[i] = c;
            }
        }
    }

    public partial class FlowWorld
    {
        static readonly (string name, int capacity)[] s_Layout = { ("a", 10), ("b", 5) };
        NativeArray<Cell> m_Cells;

        public static (int sources, int sinks) Count(int cells) => (cells / 2, cells - cells / 2);

        public void Step()
        {
            new FlowJob { cells = m_Cells, rate = 1 }.Run();
        }
    }
}
