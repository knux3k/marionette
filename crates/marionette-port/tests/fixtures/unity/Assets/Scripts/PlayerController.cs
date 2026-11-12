using System.Collections;
using UnityEngine;

namespace Demo
{
    public enum PlayerMode { Walking, Flying }

    [RequireComponent(typeof(Rigidbody), typeof(BoxCollider))]
    public class PlayerController : MonoBehaviour
    {
        [Header("Movement")]
        [SerializeField] private float speed = 5f;
        public float jumpHeight = 2.5f;
        public Vector3 spawnPoint = new Vector3(0, 1, -2);
        public GameObject target;
        public AudioClip jumpSound;
        public PlayerMode mode = PlayerMode.Walking;
        public List<int> scores;
        private Rigidbody rb;
        private int coins, lives = 3;
        private const float MaxSpeed = 10f;
        public int Score { get; private set; }

        void Awake()
        {
            rb = GetComponent<Rigidbody>();
        }

        void Start()
        {
            transform.position = spawnPoint;
            StartCoroutine(Blink());
            Invoke("Respawn", 2f);
        }

        void Update()
        {
            float h = Input.GetAxis("Horizontal");
            if (Input.GetButtonDown("Jump"))
            {
                rb.AddForce(Vector3.up * jumpHeight, ForceMode.Impulse);
            }
            transform.Translate(h * speed * Time.deltaTime, 0, 0);
            float wobble = Mathf.Sin(Time.time); // a hazard
            Debug.Log($"h={h} wobble={wobble}");
            Debug.Log("Input.GetAxis is only text here"); /* Input.GetAxis("x") */
        }

        void OnTriggerEnter(Collider other)
        {
            if (other.CompareTag("Coin"))
            {
                coins++;
                Destroy(other.gameObject);
            }
        }

        void OnDestroy() { }

        IEnumerator Blink()
        {
            yield return new WaitForSeconds(0.5f);
            int roll = Random.Range(0, 6);
            transform.childCount.ToString();
        }

        void Respawn() => transform.position = spawnPoint;
    }
}
