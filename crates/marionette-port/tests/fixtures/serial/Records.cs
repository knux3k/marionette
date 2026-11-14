using System;
using System.Collections.Generic;
using UnityEngine;

namespace Sample
{
    public enum Mode { Idle, Busy = 5, Done }

    [Flags]
    public enum Perks : byte { None = 0, Fast = 1 << 0, Strong = 1 << 1, Both = Fast | Strong }

    [Serializable]
    public class Base
    {
        public int version = 3;
        public string label = "base";
    }

    [Serializable]
    public struct Slot
    {
        public int id;
        public float amount;
    }

    [Serializable]
    public class Profile : Base
    {
        public string name = "new \"player\"";
        public float speed = 1.5f;
        public double ratio = 2;
        public long ticks = -7;
        public ulong big = 0xFF;
        public bool active = true;
        public Mode mode = Mode.Busy;
        public Perks perks;
        public Vector3 position = new Vector3(1, 2, 3);
        public Quaternion rotation = Quaternion.identity;
        public Color tint = Color.white;
        public Slot[] slots = new Slot[0];
        public List<int> ids = new List<int> { 4, 5 };
        public List<string> tags;
        public Inner inner = new Inner();
        public GameObject target;
        public Mystery other;
        public int type;
        [SerializeField] private int hidden = 9;
        private int notSaved = 1;
        [NonSerialized] public int skipped;
        public static int Count;
        public const int Max = 4;
        public Dictionary<string, int> lookup = new Dictionary<string, int>();
        public int[][] jagged;
        public Helper helper;
        public int computed = Compute();
        public int Level { get; set; }
        static int Compute() { return 1; }
    }

    public class Helper { public int x; }

    [Serializable]
    public class Inner
    {
        public int _underscore = 1;
        public int HPValue;
        public Mode[] modes;
    }

    [Serializable]
    public class Extras
    {
        public const int Current = 7;
        public static readonly string Home = "home";
        public int version = Current;
        public string where = Home;
        public float lowest = float.MinValue;
        public Inner nested = new Sample.Inner();
        public bool Ranked => version > 0 && lowest < 0;
        public List<Foreign> foreign = new List<Foreign>();
        public Foreign[] more;
        public Mystery[] things;
        public Vector3 dir = Vector3.zero;
    }

    [Serializable]
    public partial class Split { public int a; }
    public partial class Split { public int b = 2; }
}
